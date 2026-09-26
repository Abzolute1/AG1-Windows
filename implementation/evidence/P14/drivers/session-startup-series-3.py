"""Measure installed CLI initialization, complete pre-model admission and model load."""
import argparse, copy, importlib.util, json, os, subprocess, sys, threading, time, traceback
import urllib.request, urllib.error
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path.cwd()
sys.path[:0] = [str(ROOT / 'tools'), str(ROOT / 'tools/release_duration')]
from release_gate import artifact, ref, input_digest
from cli_client import NativeClient
from resources import identity, same_process, descendants
from workload_checks import history, page

def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())

def process_tree(root):
    """Observe children of every thread; reading does not grant stop authority."""
    if not same_process(root, identity(root['pid'])):
        return []
    pending = [root['pid']]
    visited = set()
    rows = []
    while pending:
        pid = pending.pop()
        if pid in visited:
            continue
        visited.add(pid)
        row = identity(pid)
        if row is None:
            continue
        rows.append(row)
        for path in Path('/proc', str(pid), 'task').glob('*/children'):
            try:
                pending.extend(map(int, path.read_text().split()))
            except (FileNotFoundError, ProcessLookupError):
                pass
    return rows

class Observer:
    def __init__(self, owner, expected, origin, output):
        self.owner, self.expected, self.origin, self.output = owner, expected, origin, output
        self.stop = threading.Event()
        self.known = {}
        self.events = []
        self.exec_ns = self.healthy_ns = None
        self.error = None
        self.max_poll_gap_ns = 0
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        previous = time.monotonic_ns()
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            while not self.stop.is_set():
                now = time.monotonic_ns()
                self.max_poll_gap_ns = max(self.max_poll_gap_ns, now - previous)
                previous = now
                for row in process_tree(self.owner):
                    self.known[row['pid']] = row
                    if self.expected and row['executable'] == self.expected['command']:
                        argv = Path('/proc', str(row['pid']), 'cmdline').read_bytes().rstrip(b'\0').split(b'\0')
                        assert argv == [s.encode() for s in [self.expected['command'], *self.expected['args']]], 'Owned model argv differs'
                        assert same_process(row, identity(row['pid']))
                        if self.exec_ns is None:
                            self.exec_ns = time.monotonic_ns()
                            self.events.append({'kind':'owned_model_exec_observed', 'monotonic_ns':self.exec_ns, 'identity':row})
                if self.exec_ns is not None and self.healthy_ns is None:
                    try:
                        with opener.open(self.origin + '/health', timeout=.25) as response:
                            body = response.read(65537)
                            assert len(body) <= 65536 and response.status == 200
                        value = json.loads(body)
                        assert value.get('status') == 'ok', value
                        self.healthy_ns = time.monotonic_ns()
                        self.events.append({'kind':'owned_endpoint_health_ready_observed', 'monotonic_ns':self.healthy_ns, 'response':value})
                    except (OSError, urllib.error.URLError):
                        pass
                self.stop.wait(.025)
        except Exception as error:
            self.error = repr(error)

    def close(self):
        self.stop.set()
        self.thread.join(timeout=5)
        assert not self.thread.is_alive()
        save(self.output / 'runtime-observation.json', {'events':self.events, 'processes':list(self.known.values()), 'max_poll_gap_ms':self.max_poll_gap_ns/1e6, 'error':self.error})

def live_milestones(began, ready, prompt, model_exec, healthy, usable):
    # Capture verification runs inside Host::open, before the first client view.
    # Include that work in admission instead of timing only the later prompt.
    assert began <= ready <= prompt < model_exec < healthy <= usable
    return {
        'preservation_admission_ms': (model_exec - began) / 1e6,
        'initialization_and_capture_ms': (ready - began) / 1e6,
        'client_configuration_ms': (prompt - ready) / 1e6,
        'prompt_to_model_exec_ms': (model_exec - prompt) / 1e6,
        'model_load_ms': (healthy - model_exec) / 1e6,
    }

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--fixture', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    assert output.parent == ROOT / '.local/p14-performance-preparation' and not output.exists()
    if not args.fixture:
        for path in Path('/proc').iterdir():
            if not path.name.isdigit():
                continue
            try:
                assert (path / 'comm').read_text().strip() != 'llama-server', 'Finish existing live owner first'
            except (FileNotFoundError, ProcessLookupError):
                pass
    package = json.load(open(ROOT / 'implementation/evidence/P14/package.json'))
    digest = input_digest()
    assert package['input_sha256'] == digest
    archive = artifact(package['archive'])
    output.mkdir()
    spec = importlib.util.spec_from_file_location('startup_installer', ROOT / 'product/release/install.py')
    installer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(installer)
    installed = installer.install(archive, package['archive']['sha256'], output / 'installation')
    live = json.load(open(ROOT / '.local/p14-long-horizon/live-4/input.json'))['live']
    model = live['models'][0]
    expected = next(m['expected'] for m in json.load(open(live['capture']['path']))['request']['models'] if m['model_id'] == model['id'])
    server = None
    requests = []
    if args.fixture:
        class Provider(BaseHTTPRequestHandler):
            def log_message(self, *args): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                requests.append(body)
                assert not body.get('tools')
                payload = b'data: {"choices":[{"index":0,"delta":{"content":"READY"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":1,"total_tokens":11}}\n\ndata: [DONE]\n\n'
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Content-Length', str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
        server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        model = {'id':'startup-fixture','provider':'fixture','api':'openai-completions','baseUrl':f'http://127.0.0.1:{server.server_port}/v1','contextWindow':8192,'maxTokens':1024,'reasoning':False,'input':['text']}
        expected = None
    count = 2 if args.fixture else 20
    start = {'fixture':args.fixture,'release_acceptance':False,'input_sha256':digest,'package_sha256':package['archive']['sha256'],'samples':count,'model':model['id'],'bound_inputs':[ref(Path(__file__).absolute()),ref(ROOT/'tools/release_duration/cli_client.py'),ref(ROOT/'tools/release_duration/resources.py'),ref(ROOT/'product/release/install.py'),ref(installed/'bin/ag1-cli')],
             'definitions':{'client_ready':'Installed CLI exec to first real client view, including Host::open capture verification; separate from graphical shell readiness.','preservation_admission':'Installed CLI exec to observed exact owned model exec. Includes initial capture verification, client initialization/configuration, selected-material rechecks and authenticated launch. This is a conservative combined pre-model interval, not an isolated hash span; component intervals are retained.','model_load':'Observed owned model exec to first observed health status ok. The native owner independently checks model identity/context before generation.','usable_session_ready':'Installed CLI exec to successful completion of the first strict READY prompt, including client configuration and all lazy preparation.'},
             'conditions':'Fresh workspace/state/native process and unloaded model per sample. Same exact installed package and preserved model settings. Warm kernel page cache; no eviction. Original user profile is read-only. Fixed two-word instruction, tools disabled, thinking off. Fixture mode measures only real CLI transport plumbing and never emits model/admission measurements.'}
    save(output/'started.json', start)
    rows = []
    try:
        for index in range(count):
            sample = output / f'sample-{index:02}'
            sample.mkdir()
            workspace = sample / 'workspace'
            init = subprocess.run([str(installed/'bin/ag1-cli'),'--init',str(workspace)], capture_output=True, timeout=30)
            (sample/'init.log').write_bytes(init.stdout+init.stderr)
            assert init.returncode == 0
            config = {'fixture':args.fixture,'models':[model] if args.fixture else live['models'],'selected':model['id'],'credential_env':None,'system_prompt':'Reply with exactly READY.','policy_mode':'disabled','runtime':None if args.fixture else {'capture':live['capture'],'process_host':live['process_host']}}
            save(sample/'config.json', config)
            client = observer = None
            result = {'index':index,'status':'running'}
            try:
                began = time.monotonic_ns()
                client = NativeClient(installed/'bin/ag1-cli',workspace,sample/'state',sample/'config.json',sample/'client')
                owner = identity(client.process.pid)
                assert owner
                observer = Observer(owner, expected, model['baseUrl'].removesuffix('/v1'), sample)
                ready = client.ready()
                ready_ns = time.monotonic_ns()
                save(sample/'initial-view.json', ready)
                client.execute({'kind':'runtime','action':'tools','args':{'names':[]}})
                client.execute({'kind':'model','action':'thinking','args':{'level':'off'}})
                prompt_ns = time.monotonic_ns()
                receipt = client.execute({'kind':'prompt','input':{'kind':'inline','text':'Reply READY.'}},seconds=1200)
                usable_ns = time.monotonic_ns()
                save(sample/'prompt-receipt.json', receipt)
                records = [page(client,'session','entry',{'id':r['id']}) for r in history(client)]
                save(sample/'records.json', records)
                assistant = next(r['message'] for r in reversed(records) if r.get('message',{}).get('role') == 'assistant')
                assert ''.join(b.get('text','') for b in assistant['content'] if b['type']=='text').strip() == 'READY'
                assert not observer.error
                result.update(status='measured',client_ready_ms=(ready_ns-began)/1e6,usable_session_ready_ms=(usable_ns-began)/1e6,clock={'exec':began,'client_view':ready_ns,'prompt':prompt_ns,'completed_probe':usable_ns})
                if not args.fixture:
                    result.update(live_milestones(began, ready_ns, prompt_ns, observer.exec_ns, observer.healthy_ns, usable_ns))
                    result['clock'].update(model_exec_observed=observer.exec_ns, model_health_observed=observer.healthy_ns)
            except Exception as error:
                result.update(status='failed',error=repr(error),traceback=traceback.format_exc())
                raise
            finally:
                if client:
                    try:
                        client.close()
                        result['cli_exit_code'] = client.process.returncode
                    except Exception as error:
                        result['cleanup_error'] = repr(error)
                        result['status'] = 'failed'
                if observer:
                    observer.close()
                    leaks = [r for r in observer.known.values() if (current := identity(r['pid'])) and same_process(r,current) and current['state'] != 'Z']
                    result['leaked_owned_children'] = leaks
                    if leaks:
                        result['status'] = 'failed'
                save(sample/'result.json',result)
            assert result['status'] == 'measured' and input_digest() == digest
            rows.append(ref(sample/'result.json'))
            print('sample',index,'completed',flush=True)
        save(output/'result.json',{'status':'fixture-passed' if args.fixture else 'measured','fixture':args.fixture,'release_acceptance':False,'input_sha256':digest,'package_sha256':package['archive']['sha256'],'samples':rows,'start_receipt':ref(output/'started.json')})
    finally:
        if server:
            server.shutdown()
            server.server_close()
            save(output/'fixture-requests.json',requests)

if __name__ == '__main__':
    main()
