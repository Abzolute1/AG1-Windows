"""User-directed Qwen stress investigation using P14's actual workload machinery.

This is not the P14 release gate: it reuses an integrity-checked historical seed,
not a current matched TS/Rust preflight. The unchanged repair verifiers, real
eight-hour clock, fault ownership, memory budget, and claim-review requirements
still apply. Results are retained separately and never promoted to acceptance.
"""
import argparse
import datetime
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools/release_duration'))
from run_duration import Run
from resources import Sampler
from faults import retain
from release_long_horizon import native_input
from release_gate import artifact, fingerprints, input_digest, ref
from common import verify_original


class Telemetry(Sampler):
    def _sample(self):
        super()._sample()
        sample = self.samples[-1]
        processes = []
        for row in sample['processes']:
            try:
                proc = Path('/proc') / str(row['pid'])
                fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
                processes.append({**row, 'cpu_ticks': int(fields[11]) + int(fields[12]),
                                  'open_fds': len(list((proc / 'fd').iterdir()))})
            except OSError:
                pass
        gpu = subprocess.run(['nvidia-smi', '--query-gpu=memory.used,utilization.gpu,temperature.gpu,power.draw',
                              '--format=csv,noheader,nounits'], capture_output=True, text=True, timeout=10)
        extra = {'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                 'monotonic_ns': sample['monotonic_ns'], 'clock_ticks_per_second': os.sysconf('SC_CLK_TCK'),
                 'processes': processes, 'gpu_mib_percent_celsius_watts': gpu.stdout.strip(),
                 'gpu_query_exit_code': gpu.returncode,
                 'home_free_bytes': os.statvfs('/home').f_bavail * os.statvfs('/home').f_frsize}
        with (self.output.parent / 'telemetry.jsonl').open('a') as stream:
            stream.write(json.dumps(extra) + '\n')
        print(json.dumps({'sample_utc': extra['utc'], 'host_present': sample['host_present'],
                          'tree_pss_mib': round(sample['pss_bytes'] / 1024**2, 1),
                          'heap_mib': round(sample['heap_bytes'] / 1024**2, 1),
                          'owned_children': sample['owned_children']}), flush=True)


class Investigation(Run):
    allow_semantic_continuation = True
    def __init__(self, args):
        os.umask(0o077)
        os.chdir(ROOT)
        self.args = args
        self.out = args.output.absolute()
        base = ROOT / '.local/p14-long-horizon'
        if self.out.exists() or self.out.parent.resolve() != base.resolve():
            raise RuntimeError('Use a new direct P14 long-horizon destination')
        self.out.mkdir(mode=0o700)
        self.client = None
        self.client_logs = []
        self.client_serial = self.serial = 0
        self.activities, self.scenarios, self.tasks, self.commands = [], [], [], []
        self.known_children = {}
        self.sampler = self.started = None
        self.before, self.pins = fingerprints(), verify_original()
        self.ts = args.seed_source.absolute() / 'ts'
        self.seed_source = json.loads((self.ts / 'seed.json').read_text())
        if self.seed_source['fixture'] or not self.seed_source['token_counts_exact'] or not 2_000_000 <= self.seed_source['token_count'] <= 3_000_000:
            raise RuntimeError('An exact live-tokenized seed is required')
        self.live = json.loads((args.seed_source / 'input.json').read_text())['live']
        self.model = next(m for m in self.live['models'] if m['id'] == 'Qwen3.6-35B-A3B-UD-Q4_K_XL')
        assert self.seed_source['model']['id'] == self.model['id']
        package = json.loads(args.package_report.read_text())
        archive = artifact(package['archive'])
        if package['input_sha256'] != input_digest():
            raise RuntimeError('Build the current product package before this investigation')
        self.package_sha = package['archive']['sha256']
        spec = importlib.util.spec_from_file_location('overnight_installer', ROOT / 'product/release/install.py')
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        installed = installer.install(archive, self.package_sha, self.out / 'installation')
        self.binary, self.cutover = installed / 'bin/ag1-cli', installed / 'bin/ag1-cutover'
        self.prepare_binary = args.binaries.absolute() / 'release-long-horizon-native'
        self.archive_binary = args.binaries.absolute() / 'release-recovery-checkpoint'
        self.desktop_binary = args.binaries.absolute() / 'ag1-desktop-visual'
        self.heap_library = args.heap_library.absolute()
        self.native = self.out / 'native'
        request = native_input(self.ts, self.native, self.live, self.model)
        value = json.loads(request.read_text())
        value['prepare_only'] = True
        # Investigate the normal desktop environment without terminating unrelated
        # GPU applications. Runtime/endpoint custody and captured launch plans stay
        # exact; this explicitly cannot establish an isolated P14 GPU benchmark.
        value['config']['runtime']['exclusive_gpu'] = False
        request = self.native / 'prepare-input.json'
        retain(request, value)
        self.run_command([str(self.prepare_binary), str(request)], self.out / 'prepare.log', 180)
        self.workspace, self.state = self.native / 'workspace', self.native / 'state'
        self.seed = json.loads((self.native / 'seed.json').read_text())
        self.specs = json.loads((self.native / 'specs.json').read_text())
        self.node = value['node']
        self.config = self.native / 'config.json'
        retain(self.config, value['config'])
        retain(self.state / 'host.json', value['config'])
        capture = json.loads(Path(self.live['capture']['path']).read_text())
        self.expected_plan = next(m['expected'] for m in capture['request']['models'] if m['model_id'] == self.model['id'])
        self.bound = [ref(p) for p in [self.binary, self.cutover, self.prepare_binary, self.archive_binary,
                                      self.desktop_binary, self.heap_library, Path(__file__).resolve()]]
        self.bound += [ref(p) for p in sorted((ROOT / 'tools/release_duration').glob('*.py'))]
        self.bound += [ref(self.ts / 'seed.json'), ref(self.config)]
        if fingerprints() != self.before:
            raise RuntimeError('Preparation changed the frozen source inputs')
        self.sampler = Telemetry(self.out / 'memory.jsonl', self.state)
        retain(self.out / 'investigation-scope.json', {
            'scope': 'User-directed overnight stress investigation; not P14 release acceptance',
            'current_matched_ts_rust_preflight': False,
            'exclusive_gpu': False,
            'gpu_environment': 'Shared desktop GPU; unrelated applications are preserved. Global GPU samples are not solely AG1 usage.',
            'repeat_prompt': 'Each trial is numbered and explicitly requests a new retrieval even when the immutable contract remains resident. Original source/verifier and required-recall checks are unchanged.',
            'seed_source': str(args.seed_source), 'externalized_tokens': self.seed['token_count'],
            'seed_token_accounting': 'Existing tokenizer count reused only for the unchanged imported artifact bytes',
            'model': self.model['id'], 'requested_active_hours': args.hours,
            'source_fingerprints': self.before, 'package': package['archive'],
            'strict_criteria': ['original repair verifier', 'required current-contract recall',
                                'unchanged verifiers', 'scoped mutations', 'bounded active context',
                                'owned crash/recovery', 'memory growth', 'retained final claim review'],
            'quality_continuation': 'An explicit semantic-only failed review may continue endurance observation; it remains failed and prevents a clean quality pass. Unexpected effects, unresolved commands and original workload verification failures remain fatal.',
            'preservation_audit': 'Direct retained runtime/source hashes and read-only authority inspection; strict exclusive-GPU cutover acceptance is not claimed.',
            'release_acceptance': False})

    def final_preservation(self):
        # The formal native cutover audit requires a globally empty GPU window.
        # This explicitly shared-desktop investigation verifies retained bytes
        # without pretending that exclusive-window acceptance occurred.
        import hashlib
        config = json.loads(self.config.read_text())
        capture_path = Path(config['runtime']['capture']['path'])
        capture = json.loads(capture_path.read_text())
        expected = {str(capture_path): config['runtime']['capture']['sha256']}
        for item in [capture['request']['profile'], config['runtime']['process_host']]:
            expected[item['path']] = item['sha256']
        for model in capture['request']['models']:
            for item in model['materials']:
                previous = expected.setdefault(item['path'], item['sha256'])
                if previous != item['sha256']:
                    raise RuntimeError('Conflicting retained runtime material identities')
        verified = []
        for name, digest in sorted(expected.items()):
            path = Path(name)
            with path.open('rb') as stream:
                before = os.fstat(stream.fileno())
                actual = hashlib.file_digest(stream, 'sha256').hexdigest()
                after = os.fstat(stream.fileno())
            signature = lambda st: (st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns, st.st_ctime_ns)
            if actual != digest or signature(before) != signature(after) or signature(after) != signature(path.stat()):
                raise RuntimeError('Preserved runtime bytes changed: ' + name)
            verified.append({'path': name, 'sha256': actual, 'bytes': after.st_size})
        self.run_command([str(self.cutover), 'inspect', '/home/alex/Projects/Cruzible'],
                         self.out / 'original-authority-inspection.json', 60)
        retain(self.out / 'final-preservation.json', {
            'status': 'passed-retained-byte-checks', 'release_acceptance': False,
            'exclusive_gpu_cutover_audit': 'not claimed; shared desktop GPU',
            'runtime_files': verified, 'original_source': verify_original(),
            'authority': ref(self.out / 'original-authority-inspection.json')})

    def activity(self, kind, work):
        print(json.dumps({'activity': kind, 'state': 'starting', 'completed_tasks': len(self.tasks),
                          'utc': datetime.datetime.now(datetime.timezone.utc).isoformat()}), flush=True)
        result = super().activity(kind, work)
        print(json.dumps({'activity': kind, 'state': 'passed', 'completed_tasks': len(self.tasks)}), flush=True)
        return result

    def task(self, index, through_renderer=False):
        spec = self.specs[index % len(self.specs)]
        original = spec['task']
        # After a cycle the contract may still be resident. The old one-shot
        # wording incorrectly claimed otherwise, encouraging reuse instead of
        # the explicit per-trial retrieval this endurance instrument measures.
        task = original.replace('After this session restart, repair', 'Repair')
        task = task.replace('The exact contract is intentionally outside the resident preview and its old artifact id may have been evicted: ', '')
        spec['task'] = (f'Independent regression trial {index + 1}. The driver changed the source again. '
                        'Perform a new artifact search during THIS trial even if you remember an earlier '
                        'matching immutable contract: this retrieval is an explicit part of the test, '
                        'not a request to repeat arbitrary verification. Then repair the current file '
                        'and run its original verifier.\n\n' + task)
        try:
            return super().task(index, through_renderer)
        finally:
            spec['task'] = original

    def control(self, action, args):
        # Same-model branch/resume must not replace an already loaded backend.
        from resources import descendants, identity, same_process
        def models():
            host = identity(self.client.process.pid)
            return [r for r in descendants(host) if Path(r['executable']).name == 'llama-server'] if host else []
        before = models()
        start = time.monotonic_ns()
        result = super().control(action, args)
        after = models()
        unchanged = len(before) == len(after) and all(any(same_process(a, b) for b in after) for a in before)
        with (self.out / 'session-runtime-continuity.jsonl').open('a') as stream:
            stream.write(json.dumps({'action': action, 'args': args, 'seconds': (time.monotonic_ns() - start) / 1e9,
                                     'before': before, 'after': after, 'same_loaded_processes': unchanged}) + '\n')
        if before and not unchanged:
            raise RuntimeError('Session navigation unexpectedly replaced the loaded model process')
        return result

    def renderer_probe(self, ui, view, desktop):
        """Exercise the real React navigation handlers with an already loaded model."""
        from resources import descendants, identity, same_process
        initial = view()['snapshot']['scope']['session']
        original_title = ui.js('document.querySelector(".chat-row.selected > button")?.title ?? null')
        if not original_title:
            raise RuntimeError('The active conversation has no selectable sidebar entry')
        model_before = [r for r in descendants(identity(desktop.pid)) if Path(r['executable']).name == 'llama-server']
        if len(model_before) != 1:
            raise RuntimeError('Navigation probe needs exactly one loaded owned model')
        observations = []
        for index in range(6):
            began = time.monotonic_ns()
            ui.wait('!!document.querySelector("button.new-chat:not(:disabled)")')
            ui.js('document.querySelector("button.new-chat").click();true')
            deadline = time.monotonic() + 30
            while view()['snapshot']['scope']['session'] == initial:
                if time.monotonic() > deadline:
                    raise RuntimeError('New chat did not select a new native conversation')
                time.sleep(.05)
            fresh = view()['snapshot']['scope']['session']
            ui.wait('!document.querySelector("button.new-chat").disabled')
            ui.js('(()=>{const b=[...document.querySelectorAll(".chat-row > button")].find(b=>b.title==='
                  + json.dumps(original_title) + ');if(!b||b.disabled)throw Error("Original chat unavailable");b.click();return true})()')
            while view()['snapshot']['scope']['session'] != initial:
                if time.monotonic() > deadline:
                    raise RuntimeError('Returning to the original chat did not settle')
                time.sleep(.05)
            model_after = [r for r in descendants(identity(desktop.pid)) if Path(r['executable']).name == 'llama-server']
            unchanged = len(model_after) == 1 and same_process(model_before[0], model_after[0])
            observations.append({'index': index, 'new_session': fresh, 'returned_session': initial,
                                 'seconds': (time.monotonic_ns() - began) / 1e9,
                                 'same_loaded_model_process': unchanged})
            if not unchanged:
                raise RuntimeError('Desktop chat navigation reloaded the model')
        ui.screenshot('chat-navigation-model-retained')
        result = {'status': 'passed', 'scope': 'Actual React handlers through native WebKit DOM input; not physical mouse input',
                  'model_before': model_before, 'observations': observations, 'model_reloads': 0}
        retain(self.out / 'desktop-chat-continuity.json', result)
        return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--seed-source', type=Path, required=True)
    parser.add_argument('--package-report', type=Path, default=ROOT / 'implementation/evidence/P14/package.json')
    parser.add_argument('--binaries', type=Path, required=True)
    parser.add_argument('--heap-library', type=Path, required=True)
    parser.add_argument('--hours', type=int, choices=[8], default=8)
    Investigation(parser.parse_args()).run()
