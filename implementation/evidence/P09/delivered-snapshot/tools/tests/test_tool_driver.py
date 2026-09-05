"""Real native P05 fixture dispatch; no model, network, or desktop processes."""
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class ToolDriverTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build = subprocess.run(
            ['cargo', 'build', '--offline', '--manifest-path', str(ROOT / 'product/Cargo.toml'),
             '-p', 'ag1-platform', '--example', 'tool-driver', '--bin', 'ag1-process-host'],
            cwd=ROOT, capture_output=True, text=True, timeout=120)
        if build.returncode:
            raise AssertionError(build.stderr)
        cls.binary = ROOT / 'product/target/debug/examples/tool-driver'

    def setUp(self):
        self.owned = tempfile.TemporaryDirectory(prefix='p09-driver-contract-', dir=ROOT / '.local')
        self.addCleanup(self.owned.cleanup)
        self.base = Path(self.owned.name)
        self.workspace = self.base / 'workspace'
        self.workspace.mkdir()
        (self.workspace / '.ag1-tool-fixture').write_text('P09 isolated tool fixture\n')

    def run_driver(self, requests):
        source = self.base / 'requests.json'
        output = self.base / 'results.jsonl'
        source.write_text(json.dumps(requests))
        run = subprocess.run([str(self.binary), str(self.workspace), str(self.base / 'store'),
                              str(source), str(output)], cwd=ROOT, capture_output=True, text=True, timeout=30)
        self.assertEqual(run.returncode, 0, run.stderr)
        rows = [json.loads(line) for line in output.read_text().splitlines()]
        self.assertEqual(len(rows), len(requests))
        self.assertTrue(all('error' not in row for row in rows), rows)
        return rows

    def test_file_dispatch_approval_replay_and_rejected_calls_use_durable_results(self):
        requests = [
            {'operation': 'write', 'name': 'write', 'arguments': {'path': 'file.txt', 'content': 'alpha\n'}, 'approve': True},
            {'operation': 'read', 'name': 'read', 'arguments': {'path': 'file.txt'}},
            {'operation': 'edit', 'name': 'edit', 'arguments': {'path': 'file.txt', 'edits': [{'oldText': 'alpha', 'newText': 'beta'}]}, 'approve': True},
            {'operation': 'edit', 'name': 'edit', 'arguments': {'path': 'file.txt', 'edits': [{'oldText': 'alpha', 'newText': 'beta'}]}, 'approve': True},
            {'operation': 'denied', 'name': 'write', 'arguments': {'path': 'denied', 'content': 'no'}, 'approve': False},
            {'operation': 'denied', 'name': 'write', 'arguments': {'path': 'denied', 'content': 'no'}, 'approve': False},
            {'operation': 'planning', 'name': 'append', 'arguments': {'path': 'file.txt', 'content': 'no'}, 'mode': 'planning'},
            {'operation': 'truncated', 'name': 'write', 'arguments': {'path': 'file.txt', 'content': 'no'}, 'complete': False},
            {'operation': 'list', 'name': 'ls', 'arguments': {}},
        ]
        rows = self.run_driver(requests)
        self.assertEqual((self.workspace / 'file.txt').read_text(), 'beta\n')
        self.assertFalse((self.workspace / 'denied').exists())
        for index in [3, 5]:
            self.assertTrue(rows[index]['replayed'])
            self.assertEqual(rows[index]['events'], [])
        self.assertEqual([row['record']['status'] for row in rows], ['completed'] * 4 + ['rejected'] * 4 + ['completed'])

    def test_search_dispatch_and_escape_recovery_are_durable_before_any_search_effect(self):
        (self.workspace / 'src').mkdir()
        (self.workspace / 'src/main.ts').write_text('export function target() {}\n')
        escape = {'operation': 'search-escape', 'name': 'grep', 'arguments': {'pattern': 'target', 'path': '/'}}
        rows = self.run_driver([
            {'operation': 'grep', 'name': 'grep', 'arguments': {'pattern': 'target', 'path': 'src'}},
            {'operation': 'find', 'name': 'find', 'arguments': {'pattern': '*.ts', 'path': 'src'}},
            escape, escape,
        ])
        self.assertEqual([r['record']['status'] for r in rows], ['completed', 'completed', 'rejected', 'rejected'])
        self.assertIn('main.ts:1: export function target()', rows[0]['result']['content'][0]['text'])
        self.assertIn('next: read src/main.ts', rows[0]['result']['content'][0]['text'])
        self.assertEqual(rows[1]['result']['details']['firstMatchPath'], 'src/main.ts')
        message = rows[2]['result']['content'][0]['text']
        self.assertIn('workspace-relative path like ., scripts, src, or packages/*/src', message)
        self.assertIn('Workspace root: ' + str(self.workspace), message)
        self.assertTrue(rows[2]['result']['details']['rejectedBeforeExecution'])
        self.assertEqual(rows[2]['result']['details']['executionEffect'], 'none')
        self.assertEqual([e['stage'] for e in rows[2]['events']], ['end'])
        self.assertTrue(rows[3]['replayed'])
        self.assertEqual(rows[3]['result'], rows[2]['result'])
        self.assertEqual(rows[3]['events'], [])

    def test_shell_dispatch_approval_cancellation_full_output_and_replay(self):
        executed = {'operation': 'shell', 'name': 'bash', 'arguments': {
            'command': "printf once >> executions; printf 'hello-shell\\n'"}, 'approve': True}
        rows = self.run_driver([
            executed, executed,
            {'operation': 'shell-denied', 'name': 'bash', 'arguments': {'command': 'printf no > denied-shell'}, 'approve': False},
            {'operation': 'shell-cancel', 'name': 'bash', 'arguments': {'command': 'printf no > cancelled-shell'}, 'approve': True, 'cancelBefore': True},
            {'operation': 'shell-lines', 'name': 'bash', 'arguments': {'command': 'seq 3000'}, 'approve': True},
        ])
        self.assertEqual((self.workspace / 'executions').read_text(), 'once')
        self.assertFalse((self.workspace / 'denied-shell').exists())
        self.assertFalse((self.workspace / 'cancelled-shell').exists())
        self.assertEqual([r['record']['status'] for r in rows], ['completed', 'completed', 'rejected', 'rejected', 'completed'])
        self.assertTrue(rows[1]['replayed'])
        self.assertEqual(rows[0]['result'], rows[1]['result'])
        self.assertEqual(rows[1]['events'], [])
        for row in rows[2:4]:
            self.assertTrue(row['result']['details']['rejectedBeforeExecution'])
            self.assertEqual(row['result']['details']['executionEffect'], 'none')
        self.assertTrue(rows[4]['result']['details']['fullOutputRetained'])
        self.assertIn('full output artifact', rows[4]['result']['content'][0]['text'])
        self.assertEqual(list((self.base / 'store/tool-spools').iterdir()), [])

    def test_path_recovery_and_filename_fallback_survive_native_rejection_and_replay(self):
        (self.workspace / 'Cyber_harness.md').write_text('# spec\n')
        (self.workspace / 'Capture d’écran.txt').write_text('selected accented filename')
        foreign = self.base / 'foreign'
        foreign.mkdir()
        (foreign / 'Cyber_harness.md').write_text('foreign original')
        escape = {'operation': 'escape', 'name': 'write', 'arguments': {
            'path': str(foreign / 'Cyber_harness.md'), 'content': 'must not publish'}, 'approve': True}
        rows = self.run_driver([
            escape, escape,
            {'operation': 'read-fallback', 'name': 'read', 'arguments': {'path': "Capture d'écran.txt"}},
            {'operation': 'read-repair', 'name': 'read', 'arguments': {'path': str(self.workspace / 'Cyber_harness.md').lstrip('/')}},
        ])
        rejected = rows[0]
        self.assertEqual(rejected['record']['status'], 'rejected')
        self.assertEqual(rejected['result']['details']['executionEffect'], 'none')
        self.assertTrue(rejected['result']['details']['rejectedBeforeExecution'])
        message = rejected['result']['content'][0]['text']
        self.assertIn('Path escapes the workspace', message)
        self.assertIn('Workspace root: ' + str(self.workspace), message)
        self.assertIn('Files with this name exist in the workspace: Cyber_harness.md.', message)
        self.assertEqual([event['stage'] for event in rejected['events']], ['end'])
        self.assertTrue(rows[1]['replayed'])
        self.assertEqual(rows[1]['result'], rejected['result'])
        self.assertEqual(rows[1]['events'], [])
        self.assertEqual(rows[2]['result']['content'][0]['text'], 'selected accented filename')
        self.assertEqual(rows[3]['result']['content'][0]['text'], '# spec\n')
        self.assertEqual((foreign / 'Cyber_harness.md').read_text(), 'foreign original')


if __name__ == '__main__':
    unittest.main()
