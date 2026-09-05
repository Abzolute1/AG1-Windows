#!/usr/bin/env python3
"""Index executable reference scenarios without claiming unexecuted tests passed."""
from __future__ import annotations
import re
from pathlib import Path
from common import ROOT, REFERENCE, EVIDENCE, GateError, read_json, sha256, write_json
import argparse

# Explicit supplements for the abbreviated test families in the research matrix.
SUPPLEMENTS = {
  4: ['agent:agent-loop', 'agent:malformed-tool-call', 'agent:request-admission'],
  9: ['agent-session-retry', 'agent-session-recovery-arbiter', 'agent-session-recovery-replay-scheduler'],
  11: ['repetition-guard', 'turn-health', 'agent:degenerate-stream'],
  17: ['agent-session-compaction-coordinator', 'compaction-serialization', 'agent-session-auto-compaction-queue'],
  18: ['micro-compact', 'compaction-summary-message'],
  20: ['session-metrics'], 21: ['prefix-shape', 'prefix-stability'],
  26: ['submit-plan-tool', 'plan-mode-utils', 'interactive-plan-commands'],
  27: ['runtime-observation', 'inspection-evidence'],
  28: ['episode-memory', 'agent-session-episode-context'],
  29: ['episode-candidate', 'isolated-model-call'],
  34: ['source-retrieval-signals', 'context-replay-trajectory'],
  37: ['repo-map', 'workspace-index'],
  41: ['artifact-store', 'artifact-tool'], 42: ['artifact-store'],
  43: ['artifact-store', 'context-compiler'], 44: ['artifact-store'],
  45: ['artifact-store', 'vision-capability'],
  46: ['session-manager/build-context', 'session-manager/tree-traversal', 'session-manager/save-entry'],
  47: ['session-manager/migration', 'session-manager/lock', 'session-manager/save-entry'],
  48: ['agent-session-branching', 'agent-session-tree-navigation', 'branch-summarization'],
  49: ['auth-storage', 'settings-manager'],
  50: ['agent-session-dynamic-provider', 'ai:openai-completions-tool-choice', 'ai:openai-codex-terminal-error', 'ai:google-gemini-cli-terminal-error'],
  51: ['auth-storage', 'ai:anthropic-oauth', 'ai:google-vertex-api-key-resolution'],
  52: ['model-registry', 'llamacpp-runtime'],
  55: ['model-capability-profile', 'thinking-capability', 'vision-capability', 'agent-session-model-switch-thinking'],
  56: ['agent:stream-watchdog', 'listener-hardening', 'nvidia-stream-fallback', 'ai:openai-completions-inline-thinking'],
  59: ['agent-session-dynamic-tools', 'extensions-runner', 'extensions-input-event'],
  61: ['tools', 'path-utils'], 62: ['file-mutation-queue', 'tools'], 63: ['tools'],
  64: ['bash-tool-output', 'bash-persist-size', 'bash-close-hang-windows'],
  65: ['job-registry', 'job-tool', 'job-wait-guard'],
  66: ['sandbox', 'bash-sandbox', 'interactive-workspace-actions'],
  67: ['check-detect', 'check-parse'],
  68: ['webfetch-ssrf', 'webfetch-credentials', 'web-tools-context'],
  69: ['visual-tools-contract', 'visual-tools', 'visual-engine', 'visual-url-policy', 'verify-ui-judge-model'],
  70: ['subagent', 'explore-tool', 'delegation-evidence'],
  71: ['resource-loader', 'skills', 'package-manager', 'extensions-discovery', 'extensions-runner'],
  72: ['skills'], 73: ['rpc-jsonl', 'rpc-validation', 'stdout-cleanliness'],
  74: ['interactive-command-actions', 'interactive-session-actions', 'interactive-settings-selector', 'interactive-runtime-settings'],
  75: ['session-metrics', 'run-diagnostics'],
  76: ['context-replay-eval', 'context-replay-trajectory'],
  77: ['context-replay-eval', 'context-replay-trajectory'],
}
FAILURE_WORDS = re.compile(r'reject|fail|abort|cancel|corrupt|invalid|missing|stale|tamper|denied|deny|error|malformed|overflow|prevent|refus|timeout|interrupted|does not|never|without', re.I)


def resolve_test(name: str) -> str:
    package, name = name.split(':', 1) if ':' in name else ('coding-agent', name)
    base = REFERENCE / 'packages' / package / 'test'
    name = name if name.endswith('.test.ts') else name + '.test.ts'
    direct = base / name
    hits = [direct] if direct.is_file() else list(base.rglob(name))
    if len(hits) != 1:
        raise GateError(f'Test anchor must resolve uniquely: {package}:{name} ({len(hits)} matches)')
    return str(hits[0].relative_to(REFERENCE))


def build():
    records = []; missing_anchors = []
    for line in (ROOT / 'research/AG1_PARITY_MATRIX.md').read_text().splitlines():
        if not re.match(r'\| A\d\d ', line): continue
        cells = [c.strip() for c in line.split('|')[1:-1]]
        ident, title = cells[0].split(' ', 1); number = int(ident[1:])
        anchors = re.findall(r'([\w-]+\.test\.ts)', cells[7]) + SUPPLEMENTS.get(number, [])
        # Paths in tests with duplicate basenames need their source directory.
        anchors = [('suite/agent-session-runtime' if a == 'agent-session-runtime.test.ts' else
                    'suite/agent-session-queue' if a == 'agent-session-queue.test.ts' else a) for a in anchors]
        paths = set()
        for anchor in anchors:
            try: paths.add(resolve_test(anchor))
            except GateError as exc: missing_anchors.append({'capability':ident, 'anchor':anchor, 'reason':str(exc)})
        if number == 78:
            paths.update(str(p.relative_to(REFERENCE)) for p in (REFERENCE / 'packages/godot-agent-runtime/test').glob('*.test.ts'))
        records.append({'id':ident, 'capability':title, 'source_owner':cells[1], 'risk':cells[6],
                        'reference_tests':[{'path':p, 'sha256':sha256(REFERENCE / p)} for p in sorted(paths)],
                        'disposition': next(r['disposition'] for r in read_json(ROOT / 'planning/coverage.json')['capabilities'] if r['id']==ident),
                        'evidence_scope': 'Executable reference scenarios; normalized whole-runtime traces are a separate fixture type.',
                        'live_gate': number in [50,51,53,54,55,69,77,78]})
    selection = {}
    for record in records:
        for row in record['reference_tests']:
            parts = Path(row['path']).parts
            selection.setdefault(parts[1], set()).add('/'.join(parts[2:]))
    write_json(ROOT / 'fixtures/catalog-selection.json', {k:sorted(v) for k,v in selection.items()})
    manifest = {'version':1,'source_commit':'e30f0b4b238ee5618928bcd17b4571eb046e7038',
                'capabilities':records,'unresolved_test_anchors':missing_anchors,
                'normalization_policy':['Only irrelevant generated IDs, timestamps and disposable roots may normalize.',
                    'Never normalize tool order, source bytes/digests, claims, outcomes, branch ancestry, model arguments or usage kinds.'],
                'captured_files': {str(p.relative_to(ROOT)):sha256(p) for p in sorted((ROOT/'fixtures/llama').glob('*.json'))}}
    write_json(ROOT / 'fixtures/manifest.json', manifest)
    print(f'{len(records)} capabilities; {sum(map(len,selection.values()))} test files; {len(missing_anchors)} unresolved exact anchors.')


def observe():
    manifest=read_json(ROOT/'fixtures/manifest.json')
    by_file={}
    for report in EVIDENCE.glob('catalog-*.json'):
        data=read_json(report)
        for file in data.get('testResults',[]):
            path=Path(file['name']).resolve().relative_to(REFERENCE.resolve()).as_posix()
            by_file[path]=[{'name':case['fullName'],'title':case.get('title',case['fullName']),
                           'status':case['status'],'report':str(report.relative_to(ROOT))}
                          for case in file.get('assertionResults',[])]
    negative=re.compile(r'reject|fail|abort|cancel|corrupt|invalid|missing|stale|tamper|denied|deny|error|malformed|overflow|prevent|refus|timeout|interrupted|does not|never|without|ignore|empty|disable|exceed|unsafe|forbidden|mismatch|throw',re.I)
    for row in manifest['capabilities']:
        cases=[case for path in row['reference_tests'] for case in by_file.get(path['path'],[])]
        passed=[c for c in cases if c['status']=='passed']
        success=next((c for c in passed if not negative.search(c['title'])),None)
        failure=next((c for c in passed if negative.search(c['title'])),None)
        row['observed_cases']=cases
        row['success_scenario']=success
        row['failure_scenario']=failure
        if row['id']=='A48':
            row['failure_scenario']={'kind':'known-defect','id':'B04','report':'implementation/evidence/P01/fork-defect.json','note':'Child read fails while parent is readable; target must fix authorized closure.'}
        row['case_form']='Executable scenario source and assertion outcome retained; no claim that a unit report is a full wire trace.'
    manifest['captured_files']={str(p.relative_to(ROOT)):sha256(p) for folder in ['llama','traces'] for p in sorted((ROOT/'fixtures'/folder).glob('*.json'))}
    write_json(ROOT/'fixtures/manifest.json',manifest)
    gaps=[r['id'] for r in manifest['capabilities'] if r['risk']=='H' and (not r['success_scenario'] or not r['failure_scenario'])]
    print(f'Observed catalog linked; high-risk paired-case gaps: {gaps}')
    if gaps:raise GateError('Missing required success/failure scenario evidence')


if __name__ == '__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--observe',action='store_true');args=parser.parse_args()
    observe() if args.observe else build()
