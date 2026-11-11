"""Build an allowlisted Linux candidate; never installs or replaces external runtimes."""
from __future__ import annotations

import gzip
import importlib.util
import json
import shutil
import tarfile
import uuid
from pathlib import Path

from common import ROOT, GateError, read_json, run, sha256, write_json
from tool_gate import environment

BINARIES = ('ag1-desktop', 'ag1-cli', 'ag1-cutover', 'ag1-process-host')
PROFILE = Path('/home/alex/.ag1/agent/llamacpp-model-profiles.json')
PROFILE_SHA = '405e79d479139866d1e7cfbffeba6dea837305efa7e41c776adef3efa8fe3f38'


def inventory(folder):
    rows = []
    for path in sorted(folder.rglob('*')):
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise GateError('Package inputs cannot contain links or special files')
        if path.is_file():
            rows.append({'path':path.relative_to(folder).as_posix(), 'sha256':sha256(path),
                         'bytes':path.stat().st_size, 'mode':path.stat().st_mode & 0o777})
    return rows


def archive(folder, output):
    # Stable metadata and gzip header: candidate bytes vary only with packaged inputs.
    with output.open('xb') as raw, gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=0) as gz, tarfile.open(fileobj=gz, mode='w') as tar:
        for path in sorted(folder.rglob('*')):
            if path.is_file():
                info = tar.gettarinfo(str(path), arcname=path.relative_to(folder).as_posix())
                info.uid = info.gid = 0
                info.uname = info.gname = ''
                info.mtime = 0
                with path.open('rb') as stream:
                    tar.addfile(info, stream)


def package():
    from release_gate import EVIDENCE, fingerprints, input_digest, ref
    if sha256(PROFILE) != PROFILE_SHA:
        raise GateError('Preserved profile changed')
    bound = ROOT / 'product/target/release/ag1-process-host'
    before_supervisor = sha256(bound)
    entry = read_json(EVIDENCE/'accepted-entry.json')
    if before_supervisor != entry['bound_supervisor_sha256']:
        raise GateError('Active bound supervisor changed')
    node, env = environment()
    target = ROOT / '.local/p14-target'
    env['CARGO_TARGET_DIR'] = str(target)
    output = ROOT / '.local/p14-packages' / uuid.uuid4().hex
    output.mkdir(parents=True)
    logs = EVIDENCE / 'package' / output.name
    commands = []
    commands.append(run(['cargo','build','--offline','--locked','--release','--manifest-path','product/Cargo.toml',
                         '-p','ag1-desktop','-p','ag1-cli','-p','ag1-platform','--bins'], env=env, log=logs/'build.log', timeout=3600))
    env['AG1_BUNDLE_AUDIT_OUTPUT'] = str(logs/'renderer-audit.json')
    commands.append(run([str(node),'tools/audit_desktop_bundle.mjs'], env=env, log=logs/'renderer-audit.log'))
    renderer = read_json(logs/'renderer-audit.json')
    stage = output/'linux-x86_64'
    (stage/'bin').mkdir(parents=True)
    for name in BINARIES:
        shutil.copyfile(target/'release'/name, stage/'bin'/name)
        (stage/'bin'/name).chmod(0o755)
    # No .local, reference checkout, TS AgentSession, browser, model or GPU payload inputs.
    shutil.copytree(ROOT/'product/licenses', stage/'licenses')
    shutil.copyfile(ROOT/'product/LICENSE', stage/'LICENSE')
    shutil.copyfile(ROOT/'fixtures/provenance/DYAD-NOTICE', stage/'NOTICE')
    for name in ('README.md','install.py','install-systemwide.py'):
        shutil.copyfile(ROOT/'product/release'/name, stage/name)
    for name in ('COMPATIBILITY.md','CUTOVER.md'):
        shutil.copyfile(ROOT/'product/clients'/name, stage/name)
    shutil.copytree(ROOT/'product/apps/desktop/src-tauri/icons', stage/'icons')
    # Explicit launch root, persistent state outside immutable installation; no profile overrides.
    launcher = stage/'run-ag1'
    launcher.write_text('#!/bin/sh\nset -eu\nAG1_INSTALL_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\nexport AG1_INSTALL_ROOT\n: "${AG1_DESKTOP_STATE:=${XDG_STATE_HOME:-$HOME/.local/state}/ag1/desktop}"\nexport AG1_DESKTOP_STATE\nexec "$AG1_INSTALL_ROOT/bin/ag1-desktop" "$@"\n')
    launcher.chmod(0o755)
    capture = read_json(ROOT/'implementation/evidence/P13/runtime-configuration-import.json')
    runtime = {'version':1,'policy':'external-read-only; no install/update writes, no stock fallback',
               'profile':{'path':str(PROFILE),'sha256':PROFILE_SHA},
               'plan_sha256':capture['plan_sha256'],
               'executables':['/home/alex/Projects/llama-cpp-turboquant/build/bin/llama-server',
                              '/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715/llama-server',
                              '/home/alex/Projects/llama-cpp-qwen4exp/build/bin/llama-server'],
               'active_supervisor':{'path':str(bound),'sha256':before_supervisor},
               'candidate_supervisor_sha256':sha256(stage/'bin/ag1-process-host'),
               'adoption':'Candidate installation grants no model custody or configuration transition.'}
    write_json(stage/'external-runtimes.json', runtime)
    # Actual native loader closure is recorded as external system requirements, never copied.
    dependencies = {}
    for name in BINARIES:
        log = logs/(name+'-ldd.log')
        commands.append(run(['ldd',str(stage/'bin'/name)], log=log))
        text = log.read_text()
        if 'not found' in text:
            raise GateError('Candidate has unresolved native shared libraries')
        dependencies[name] = []
        for line in text.splitlines():
            for word in line.split():
                if word.startswith('/') and Path(word).is_file():
                    dependencies[name].append({'path':word,'sha256':sha256(Path(word))})
    cargo = read_json(ROOT/'product/licenses/manifest.json')
    # License inventory is conservative (includes build/other-platform crates), not a claim
    # that every lockfile crate was linked. Link inputs and dynamic closure are separate.
    components = [{'type':'library','name':p['name'],'version':p['version'],
                   'licenses':[{'license':{'name':p['license']}}],
                   'properties':[{'name':'ag1:scope','value':p.get('scope','conservative Cargo dependency/license inventory')}]}
                  for p in cargo['packages']]
    components += [{'type':'library','name':p['name'],'version':p['version'],
                    'licenses':[{'license':{'name':p.get('declared_license') or 'NOASSERTION'}}]}
                   for p in renderer['licenses']]
    write_json(stage/'sbom.cdx.json', {'bomFormat':'CycloneDX','specVersion':'1.6','version':1,
               'components':components,'properties':[{'name':'ag1:binary-closure','value':'binary-dependencies.json'},
                {'name':'ag1:scope','value':'local Linux candidate; conservative static inventory; system shared libraries external'}]})
    write_json(stage/'binary-dependencies.json',dependencies)
    # Shared installs must be readable by every desktop user.
    for path in stage.rglob('*'):
        if path.is_file():
            path.chmod(0o755 if path.stat().st_mode & 0o111 else 0o644)
    inputs = fingerprints()
    manifest = {'version':1,'platform':'linux-x86_64','input_sha256':input_digest(),
                'files':inventory(stage),'optional_dependencies':{'extensions':'separate archive; external Node 24',
                'automation':'existing unsupported/reference-only dispositions; not bundled',
                'browser':'external optional','project_toolchains':'external','models':'external preserved builds'},
                'release_accepted':False}
    write_json(stage/'package-manifest.json',manifest)
    (stage/'package-manifest.json').chmod(0o644)
    tar = output/'ag1-0.1.0-rc-linux-x86_64.tar.gz'
    archive(stage,tar)
    companion = output/'extensions'
    shutil.copytree(ROOT/'product/compat/extensions',companion)
    # The extension distribution contains only its admitted bridge/loader, no parent session.
    extension_tar=output/'ag1-0.1.0-rc-extensions.tar.gz'
    archive(companion,extension_tar)
    commands.append(run([str(stage/'bin/ag1-cli'),'--version'], log=logs/'cli-smoke.log'))
    commands.append(run([str(stage/'bin/ag1-cutover'),'inspect','/home/alex/Projects/Cruzible'], log=logs/'authority-inspect.json'))
    if sha256(PROFILE) != PROFILE_SHA or sha256(bound) != before_supervisor or fingerprints() != inputs:
        raise GateError('Packaging changed preserved runtime or source inputs')
    report = {'status':'built','release_accepted':False,'platform':'linux-x86_64',
              'input_sha256':input_digest(),'archive':ref(tar),'extensions_archive':ref(extension_tar),
              'manifest':ref(stage/'package-manifest.json'),'sbom':ref(stage/'sbom.cdx.json'),
              'installed_bytes':sum(r['bytes'] for r in manifest['files']),
              'archive_bytes':tar.stat().st_size,'commands':commands,
              'signing':'unsigned local review candidate','artifacts':
              [ref(tar),ref(extension_tar),ref(stage/'package-manifest.json'),ref(stage/'sbom.cdx.json'),ref(logs/'renderer-audit.json')] + [ref(ROOT/c['log']) for c in commands]}
    write_json(EVIDENCE/'package.json',report)
    return report


def validate_candidate(report, expected_digest, root, destination):
    from release_gate import artifact
    from common import GateError
    if report.get('status') != 'built' or report.get('input_sha256') != expected_digest:
        raise GateError('Candidate is not built from the current release inputs')
    records = report.get('artifacts', [])
    inventory = {}
    for row in records:
        artifact(row, root)
        if row['path'] in inventory:
            raise GateError('Duplicate candidate artifact reference')
        inventory[row['path']] = row['sha256']
    commands = report.get('commands', [])
    if not commands:
        raise GateError('Candidate has no retained build/inspection commands')
    for command in commands:
        if type(command.get('exit_code')) is not int or command['exit_code'] != 0 or not command.get('argv') or command.get('log') not in inventory:
            raise GateError('Candidate build/inspection logs are missing or failed')
    for key in ('archive','extensions_archive','manifest','sbom'):
        row = report.get(key, {})
        if not row or inventory.get(row.get('path')) != row.get('sha256'):
            raise GateError('Candidate payload/manifest/SBOM is not bound to its build record')
    manifest = json.loads(artifact(report['manifest'],root).read_text())
    if manifest.get('input_sha256') != expected_digest or manifest.get('platform') != report.get('platform'):
        raise GateError('Candidate manifest declares a different source or platform')
    archive = artifact(report['archive'], root)
    spec = importlib.util.spec_from_file_location('p14_candidate_installer', root/'product/release/install.py')
    installer = importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
    installed = installer.install(archive, report['archive']['sha256'], destination)
    # The actual installer checks every archive member, source file hash, mode
    # and declared path before activating a fresh isolated installation.
    if (installed/'package-manifest.json').read_bytes() != artifact(report['manifest'], root).read_bytes():
        raise GateError('Installed candidate manifest differs from its retained build manifest')
    if (installed/'sbom.cdx.json').read_bytes() != artifact(report['sbom'],root).read_bytes():
        raise GateError('Installed candidate SBOM differs from its retained build record')
    return installed


def select_candidate():
    from release_gate import EVIDENCE,input_digest,ref
    current = input_digest()
    path = EVIDENCE/'package.json'
    if not path.exists() or read_json(path).get('input_sha256') != current:
        return package()
    report = read_json(path)
    output = ROOT/'.local/p14-package-validation'/uuid.uuid4().hex
    output.parent.mkdir(parents=True,exist_ok=True)
    installed = validate_candidate(report,current,ROOT,output)
    if sha256(PROFILE) != PROFILE_SHA:
        raise GateError('Preserved runtime profile changed')
    runtime = read_json(installed/'external-runtimes.json')
    bound = runtime['active_supervisor']
    if sha256(Path(bound['path'])) != bound['sha256']:
        raise GateError('Preserved active process supervisor changed')
    write_json(output/'validation.json',{'status':'passed','input_sha256':current,'archive':report['archive'],
        'installed_manifest':ref(installed/'package-manifest.json'),
        'scope':'Fresh full installer verification of the exact retained candidate; no replacement build substituted for measured bytes.'})
    return {**report,'candidate_validation':ref(output/'validation.json'),'artifacts':report['artifacts']+[ref(output/'validation.json'),ref(installed/'package-manifest.json')]}


if __name__ == '__main__':
    package()
