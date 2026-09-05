#!/usr/bin/env python3
"""Execute phase checks. A plan check never certifies implementation."""
from __future__ import annotations
import argparse
import datetime
import json
from pathlib import Path
import shutil
import sys

from common import ROOT, REFERENCE, EVIDENCE, GateError, read_json, write_json, sha256, run, verify_original
from reference_oracles import input_fingerprints, validate_vitest, validate_catalog_report
from runtime_manifest import stable_hash


def verify_artifacts(root: Path, artifacts: list[dict]):
    if not artifacts: raise GateError('Empty artifact manifest')
    seen=set()
    for artifact in artifacts:
        path=(root/artifact['path']).resolve()
        if not path.is_relative_to(root.resolve()) or path in seen:
            raise GateError('Artifact path escapes root or is duplicated')
        seen.add(path)
        if not path.is_file(): raise GateError(f'Missing artifact: {artifact["path"]}')
        if sha256(path)!=artifact['sha256']: raise GateError(f'Stale artifact: {artifact["path"]}')


def verify_runtime_material(runtime: dict, cache: dict):
    """A prior capture alone cannot certify the files still in use today."""
    material=[(Path(row['path']),row['sha256']) for row in runtime['weights']]
    material.extend((Path(path),row['sha256']) for path,row in runtime['loader_objects'].items())
    for observed in runtime.get('observed_external_processes',[]):
        material.extend((Path(row['path']),row['sha256']) for row in observed['mapped_libraries'])
    if not material:raise GateError('Empty runtime material manifest')
    for path,expected in material:
        digest,_=stable_hash(path,cache)
        if digest!=expected:raise GateError(f'Runtime/weight identity changed: {path}')


def check_plan():
    graph=read_json(ROOT/'planning/phase-graph.json')
    visited=set()
    for phase,row in sorted(graph.items()):
        if not set(row['dependencies'])<=visited: raise GateError(f'Impossible dependency order: {phase}')
        if not (ROOT/row['file']).is_file(): raise GateError(f'Missing phase document: {phase}')
        visited.add(phase)
    print(json.dumps({'mode':'plan-check','phase_documents':len(visited),'implementation_certified':False}))


def verify_dependencies(phase):
    graph=read_json(ROOT/'planning/phase-graph.json')
    if phase not in graph:raise GateError(f'Unknown phase: {phase}')
    for dependency in graph[phase]['dependencies']:
        report=read_json(ROOT/f'implementation/evidence/{dependency}/completion.json')
        if report.get('phase')!=dependency or report.get('status')!='passed':raise GateError(f'Dependency not passed: {dependency}')
        verify_artifacts(ROOT,report.get('artifacts',[]))


def validate_p01():
    required=['fixtures/child-inventory.json','fixtures/schemas/manifest.schema.json',
              'fixtures/schemas/session-capture.schema.json','fixtures/provenance/package-evidence.json',
              'implementation/evidence/P01/bootstrap.json','reference/archive.json']
    for path in required:
        if not (ROOT/path).is_file():raise GateError(f'Missing phase deliverable: {path}')
        read_json(ROOT/path)
    bootstrap=read_json(EVIDENCE/'bootstrap.json')
    if sha256(ROOT/bootstrap['archive'])!=bootstrap['archive_sha256']:raise GateError('Immutable source archive changed')
    fingerprints=input_fingerprints()
    for suite in ['selected','known-defects','llama','session','catalog']:
        report=read_json(EVIDENCE/f'oracle-{suite}.json')
        if report.get('status')!='passed' or report.get('input_fingerprints')!=fingerprints:
            raise GateError(f'Stale, incomplete or failed oracle: {suite}')
    validate_vitest(read_json(EVIDENCE/'coding-agent-tests.json'),expected=465)
    validate_vitest(read_json(EVIDENCE/'godot-agent-runtime-tests.json'),expected=26)
    selection=read_json(ROOT/'fixtures/catalog-selection.json')
    exceptions=read_json(ROOT/'fixtures/catalog-exceptions.json')['exceptions']
    for package,paths in selection.items():
        data=read_json(EVIDENCE/f'catalog-{package}.json')
        actual={Path(f['name']).resolve().relative_to(REFERENCE/'packages'/package).as_posix() for f in data['testResults']}
        if actual!=set(paths):raise GateError(f'Catalog files not fully executed: {package}')
        validate_catalog_report(data,exceptions)
    manifest=read_json(ROOT/'fixtures/manifest.json')
    if [r['id'] for r in manifest['capabilities']]!=[f'A{i:02}' for i in range(1,86)]:raise GateError('Capability IDs missing/duplicated')
    if manifest.get('unresolved_test_anchors'):raise GateError('Unresolved fixture anchors')
    for row in manifest['capabilities']:
        for test in row['reference_tests']:
            if sha256(REFERENCE/test['path'])!=test['sha256']:raise GateError('Stale test fixture')
        if row['risk']=='H' and (not row.get('success_scenario') or not row.get('failure_scenario')):
            raise GateError(f'High-risk fixture evidence incomplete: {row["id"]}')
    verify_artifacts(ROOT,[{'path':p,'sha256':h} for p,h in manifest['captured_files'].items()])
    runtime=read_json(EVIDENCE/'runtime-manifest.json')
    if runtime['missing_models'] or runtime['unresolved_static_dependencies'] or any(runtime['model_actions'].values()):
        raise GateError('Runtime capture missing facts or performed a model action')
    if len(runtime['weights'])!=30:raise GateError('Expected the three pinned models including 28 Flash shards')
    verify_runtime_material(runtime,read_json(ROOT/'.local/runtime/hash-cache.json'))
    lock=read_json(ROOT/'planning/evidence/llama-runtime-lock.json')
    if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('User runtime profile changed')
    packages=read_json(ROOT/'fixtures/provenance/package-evidence.json')
    for package in packages:
        verify_artifacts(ROOT,[{'path':r['path'],'sha256':r['sha256']} for r in package['license_files']])
    ledger=read_json(ROOT/'fixtures/provenance/dyad-extraction-ledger.json')
    if not ledger['entries']:raise GateError('Missing frontend provenance candidates')
    for candidate in ledger['entries']:
        if sha256(ROOT/'vendor/dyad'/candidate['source'])!=candidate['sha256']:raise GateError('Stale Dyad candidate')
        for file in candidate['closure']['source_files']:
            if sha256(ROOT/'vendor/dyad'/file['path'])!=file['sha256']:raise GateError('Stale transitive source')
        if candidate['closure']['restricted_dependencies'] and not candidate['disposition'].startswith('excluded'):
            raise GateError('Restricted dependency cannot be approved')
    return fingerprints


def execute_p01():
    EVIDENCE.mkdir(parents=True,exist_ok=True)
    completion=EVIDENCE/'completion.json'
    write_json(completion,{'phase':'P01','status':'running'})
    commands=[]
    try:
        verify_original()
        # These are required behavioral tests of the harness, not checklist checks.
        import unittest
        discovered=unittest.defaultTestLoader.discover(str(ROOT/'tools/tests'))
        if discovered.countTestCases()<16:raise GateError('Harness test selection is empty or incomplete')
        commands.append(run(['python3','-m','unittest','discover','-s','tools/tests','-v'],log=EVIDENCE/'harness-tests.log'))
        for suite in ['selected','known-defects','llama','session','catalog']:
            print(f'P01: executing {suite}',flush=True)
            commands.append(run(['python3','tools/reference_oracles.py','--suite',suite,'--offline'],log=EVIDENCE/f'gate-{suite}.log',timeout=1200))
        commands.append(run(['python3','tools/build_fixture_manifest.py','--observe'],log=EVIDENCE/'fixture-observation.log'))
        fingerprints=validate_p01()
        snapshot=EVIDENCE/'harness-snapshot';snapshot.mkdir(exist_ok=True)
        for file in (ROOT/'tools').rglob('*'):
            if file.is_file() and file.suffix in ['.py','.mjs']:
                target=snapshot/file.relative_to(ROOT/'tools');target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(file,target)
        files=[p for folder in [ROOT/'fixtures',EVIDENCE] for p in folder.rglob('*') if p.is_file() and p!=completion and '__pycache__' not in p.parts]
        artifacts=[{'path':str(p.relative_to(ROOT)),'sha256':sha256(p)} for p in sorted(set(files))]
        verify_artifacts(ROOT,artifacts)
        catalog=read_json(EVIDENCE/'oracle-catalog.json')['result']
        pending=['15 credential-dependent/Windows source tests remain pending live/platform gates; none counted passed.',
                 'R01 original prefix assertion remains failing with exact classification and independently passing replacement trace.',
                 'Per-model live loader verification, deployment environment, quality and performance remain P06/P13/P14 gates.',
                 'Frontend candidates are not approved production bundles; visual and compiler/transitive asset closure review belongs to P04.']
        write_json(completion,{'phase':'P01','status':'passed','completed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                              'input_fingerprints':fingerprints,'dependencies':[], 'artifacts':artifacts,'commands':commands,
                              'coverage':{'capabilities':85,'selected_tests':491,'catalog':catalog,'llama_launch_plans':9,'request_payloads':6,'session_scenarios':2,'known_defects':4},
                              'authority':{'before':'Original AG1 and externally owned llama processes','after':'Unchanged; isolated fixtures and harness only'},
                              'pending_gates':pending,'product_implementation_started':False})
        print(f'P01 passed: 491 reference tests; expanded catalog {catalog}; runtime preserved.')
    except Exception as exc:
        write_json(completion,{'phase':'P01','status':'failed','commands':commands,'error':str(exc),'authority':'Original sources/runtimes unchanged'})
        raise


def main():
    parser=argparse.ArgumentParser();parser.add_argument('phase',nargs='?');parser.add_argument('--check-plan',action='store_true');args=parser.parse_args()
    if args.check_plan:check_plan();return
    if not args.phase:raise GateError('A phase is required')
    verify_dependencies(args.phase)
    if args.phase=='P01':execute_p01()
    elif args.phase=='P02':
        from protocol_gate import execute_p02
        execute_p02()
    elif args.phase=='P03':
        from platform_gate import execute_p03
        execute_p03()
    elif args.phase=='P04':
        from desktop_gate import execute_p04
        execute_p04()
    elif args.phase=='P05':
        from storage_gate import execute_p05
        execute_p05()
    else:raise GateError(f'{args.phase} has no implemented gate yet; refusing a placeholder pass')


if __name__=='__main__':
    try:main()
    except GateError as exc:
        print(f'FAILED: {exc}',file=sys.stderr);raise SystemExit(1)
