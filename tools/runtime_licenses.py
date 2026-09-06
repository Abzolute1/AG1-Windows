"""P10 pinned source provenance and retained notices; no network lookup."""
import json
import re
from common import ROOT, REFERENCE, PIN, GateError, read_json, sha256
from tool_licenses import verify as tool_licenses

def verify():
    dependencies = tool_licenses()
    manifest = read_json(ROOT/'product/licenses/p10-runtime-provenance.json')
    if manifest.get('source_commit') != PIN or manifest.get('license') != 'MIT':
        raise GateError('Unpinned runtime provenance')
    if sha256(ROOT/'product/LICENSE') != sha256(REFERENCE/'LICENSE'):
        raise GateError('Missing original AG1 notice')
    sources = {}
    for oracle in (ROOT/'fixtures/runtime').glob('*oracle.json'):
        for row in read_json(oracle).get('source_files', []):
            sources[row['path']] = row['sha256']
    if not sources or manifest.get('source_files') != sources:
        raise GateError('Incomplete runtime source provenance')
    for path,digest in sources.items():
        source = (REFERENCE/path).resolve()
        if not source.is_relative_to(REFERENCE) or sha256(source) != digest:
            raise GateError('Changed runtime source: '+path)
    destinations = manifest.get('destinations',{})
    expected = {p.relative_to(ROOT).as_posix() for p in (ROOT/'product/crates').rglob('*')
                if p.is_file() and (('/src/runtime/' in p.as_posix()) or '/src/session/' in p.as_posix()
                    or p.name in ['session.rs','runtime_recovery.rs','runtime_selection.rs','session_lifecycle.rs','artifact_catalog.rs','artifact_search.rs'])}
    if not expected <= destinations.keys():
        raise GateError('Missing native runtime port provenance')
    for path,digest in destinations.items():
        target = (ROOT/path).resolve()
        if not target.is_relative_to(ROOT/'product') or sha256(target) != digest:
            raise GateError('Changed native runtime provenance: '+path)
    branch = (REFERENCE/'packages/coding-agent/src/core/compaction/branch-summarization.ts').read_text()
    prompt = re.search(r'const BRANCH_SUMMARY_PROMPT = `([^`]+)`;',branch)
    if not prompt or prompt[1] != (ROOT/'product/crates/ag1-core/src/context/prompts/branch_summarization_prompt.txt').read_text().rstrip('\n'):
        raise GateError('Branch prompt differs from pinned source')
    if read_json(ROOT/'product/crates/ag1-core/resources/runtime-task-tool.json') != read_json(ROOT/'fixtures/runtime/task-tool-oracle.json')['metadata']:
        raise GateError('Task tool metadata drift')
    if (ROOT/'product/crates/ag1-core/resources/runtime-planning-guidance.txt').read_text().strip() != read_json(ROOT/'fixtures/runtime/plan-prompt-oracle.json')['guidance']:
        raise GateError('Planning guidance drift')
    return {'status':'passed','dependencies':dependencies,'source_commit':PIN,'source_files':sources,
            'native_destinations':destinations,'notice':{'path':'product/LICENSE','sha256':sha256(ROOT/'product/LICENSE')},
            'copied_dyad_files':0}
