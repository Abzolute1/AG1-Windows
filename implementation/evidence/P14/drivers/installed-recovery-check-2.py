"""One disposable installed Linux archive/recovery check; no duration or model run."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tools'))
sys.path.insert(0, str(ROOT / 'tools/release_duration'))
from release_gate import artifact, input_digest, ref
from release_duration.cli_client import NativeClient
from release_duration.workload_checks import history, page


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    os.chdir(ROOT)
    os.umask(0o077)
    out = args.output.absolute()
    assert out.parent == ROOT / '.local/p14-long-horizon' and not out.exists()
    package = json.loads((ROOT / 'implementation/evidence/P14/package.json').read_text())
    digest = input_digest()
    assert package['input_sha256'] == digest
    archive = artifact(package['archive'])
    helper = ROOT / '.local/p14-quality-target/release/examples/release-recovery-checkpoint'
    helper_binding = ref(helper)
    out.mkdir()
    spec = importlib.util.spec_from_file_location('recovery_installer', ROOT / 'product/release/install.py')
    installer = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(installer)
    installed = installer.install(archive, package['archive']['sha256'], out / 'installation')
    binary = installed / 'bin/ag1-cli'
    commands = []

    def run(argv, log, timeout=120):
        result = subprocess.run([str(a) for a in argv], capture_output=True, timeout=timeout)
        log.write_bytes(result.stdout + result.stderr)
        commands.append({'argv': [str(a) for a in argv], 'exit_code': result.returncode,
                         'log': str(log.relative_to(ROOT))})
        assert result.returncode == 0, log
        return json.loads(result.stdout) if result.stdout.strip().startswith(b'{') else None

    workspace, state = out / 'workspace', out / 'state'
    run([binary, '--init', workspace], out / 'init.log')
    requests = []

    class Provider(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            requests.append(body)
            assert not body.get('tools')
            payload = b'data: {"choices":[{"index":0,"delta":{"content":"RECOVERY-CHECK-READY"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n'
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

    server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    # Archive closure includes the standard state-local host configuration.
    # The first fixture used an external CLI config and retained its NotFound
    # refusal; do not make archive_native infer or copy external configuration.
    state.mkdir()
    config = state / 'host.json'
    save(config, {'fixture': True, 'models': [{'id': 'recovery-check', 'provider': 'fixture',
         'api': 'openai-completions', 'baseUrl': f'http://127.0.0.1:{server.server_port}/v1',
         'contextWindow': 8192, 'maxTokens': 1024, 'reasoning': False, 'input': ['text']}],
         'selected': 'recovery-check', 'credential_env': None, 'system_prompt': 'Reply READY.',
         'policy_mode': 'disabled', 'runtime': None})
    save(out / 'started.json', {'input_sha256': digest, 'package_sha256': package['archive']['sha256'],
         'installed_cli': ref(binary), 'archive_helper': helper_binding,
         'archive_helper_source': ref(ROOT / 'product/crates/ag1-platform/examples/release-recovery-checkpoint.rs'),
         'driver': ref(Path(__file__).resolve()), 'duration_run': False, 'live_model_requests': 0,
         'scope': 'Actual installed CLI and native archive API in fresh isolated workspace authority. Scripted transport only; production state and runtime are not opened.'})
    client = None
    try:
        client = NativeClient(binary, workspace, state, config, out / 'client-before')
        client.ready()
        client.execute({'kind': 'runtime', 'action': 'tools', 'args': {'names': []}})
        receipt = client.execute({'kind': 'prompt', 'input': {'kind': 'inline', 'text': 'Record the recovery marker.'}})
        before = [page(client, 'session', 'entry', {'id': r['id']}) for r in history(client)]
        save(out / 'records-before.json', before)
        save(out / 'prompt-receipt.json', receipt)
        assert 'RECOVERY-CHECK-READY' in json.dumps(before) and len(requests) == 1
        client.close()
        client = None
        request = out / 'archive-request.json'
        save(request, {'workspace': str(workspace), 'archive': str(out / 'native-archive')})
        saved = run([helper, request], out / 'archive.log')
        sha = saved['receipt']['cutover_recovery_sha256']
        database = state / 'store/storage.sqlite'
        with database.open('r+b') as stream:
            stream.write(b'P14 CORRUPTED DB\0')
            stream.flush()
            os.fsync(stream.fileno())
        broken = ref(database)['sha256']
        refused = subprocess.run([str(binary), '--workspace', str(workspace), '--state', str(state),
                   '--config', str(config), '--mode', 'jsonl'], input=b'', capture_output=True, timeout=120)
        (out / 'refused.log').write_bytes(refused.stdout + refused.stderr)
        assert refused.returncode != 0 and b'NotADatabase' in refused.stderr
        assert ref(database)['sha256'] == broken
        save(out / 'refused-startup.json', {'exit_code': refused.returncode, 'database_sha256': broken,
             'corrupt_bytes_unchanged': True, 'log': ref(out / 'refused.log')})
        request = out / 'restore-request.json'
        save(request, {'workspace': str(workspace), 'archive': str(out / 'native-archive'),
             'restore_sha256': sha, 'retained_corrupt_state': str(out / 'corrupt-state'),
             'restored_staging': str(out / 'restored-staging')})
        run([helper, request], out / 'restore.log')
        assert ref(out / 'corrupt-state/store/storage.sqlite')['sha256'] == broken
        client = NativeClient(binary, workspace, state, config, out / 'client-after')
        client.ready()
        after = [page(client, 'session', 'entry', {'id': r['id']}) for r in history(client)]
        save(out / 'records-after.json', after)
        assert after == before and len(requests) == 1
        assert client.view['snapshot']['status'] == 'idle'
        assert client.view['snapshot']['active_operation'] is None
        client.close()
        client = None
        artifact(helper_binding)
        assert input_digest() == digest
        save(out / 'result.json', {'status': 'passed', 'release_acceptance': False,
             'duration_run': False, 'fixture': True, 'live_model_requests': 0,
             'scripted_requests': len(requests), 'records_restored_exactly': len(before),
             'corrupt_startup_refused_without_rewrite': True, 'corrupt_database_retained': True,
             'unsolicited_replay': False, 'authority_transferred': False,
             'cli_drains': [0, 0], 'input_sha256': digest,
             'package_sha256': package['archive']['sha256'], 'commands': commands})
        print('Installed native archive recovery passed; no live model or duration run.')
    finally:
        if client is not None:
            client.close()
        server.shutdown()
        server.server_close()
        save(out / 'scripted-requests.json', requests)


if __name__ == '__main__':
    main()
