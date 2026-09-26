from pathlib import Path
import sys,json,hashlib,importlib.util,os
root=Path('/home/alex/Projects/AG1');base=root/'.local/mcp-c1';destination=Path('/home/alex/.local/share/ag1/application')
sys.path.insert(0,str(root/'tools'));import release_gate
assert json.loads((base/'production-inspection.json').read_text())['status']=='inspected'
assert json.loads((base/'production-input-verification.json').read_text())['unchanged']
report=json.loads((base/'package.json').read_text());assert release_gate.input_digest()==report['input_sha256']
protected=json.loads((base/'preservation-start.json').read_text())
def sha(p):
 p=Path(p);p=p if p.is_absolute() else root/p
 return hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
assert all(sha(p)==h for p,h in protected.items())
previous=(destination/'current').resolve();system=Path('/opt/ag1/current').resolve();launcher=Path('/home/alex/.local/share/applications/ag1.desktop');wrapper=Path('/home/alex/.local/bin/ag1-desktop')
before={'previous_user_release':str(previous),'system_release':str(system),'launcher_sha256':sha(launcher),'wrapper_sha256':sha(wrapper)}
with (base/'user-install-before.json').open('x') as f:json.dump(before,f,indent=2)
assert not (destination/'pending.json').exists(),'Inspect existing pending installation; never retry blindly'
spec=importlib.util.spec_from_file_location('ag1_installer',root/'product/release/install.py');installer=importlib.util.module_from_spec(spec);spec.loader.exec_module(installer)
release=installer.install(root/report['archive']['path'],report['archive']['sha256'],destination)
manifest=json.loads((release/'package-manifest.json').read_text())
for row in manifest['files']:
 p=release/row['path'];assert sha(p)==row['sha256'],p;assert p.stat().st_size==row['bytes'],p
assert (destination/'current').resolve()==release
receipt=json.loads((destination/'receipts'/(release.name+'.json')).read_text());assert receipt['archive_sha256']==report['archive']['sha256'];assert receipt['previous']=='releases/'+previous.name
assert Path('/opt/ag1/current').resolve()==system and sha(launcher)==before['launcher_sha256'] and sha(wrapper)==before['wrapper_sha256']
assert all(sha(p)==h for p,h in protected.items())
verified={'status':'installed_and_verified','scope':'per-user production installation','release':str(release),'verified_payload_files':len(manifest['files']),'desktop_sha256':sha(release/'bin/ag1-desktop'),'archive_sha256':report['archive']['sha256'],'receipt':receipt,'preserved_entries':len(protected),'previous_user_release':str(previous),'system_release_unchanged':str(system),'launcher':str(launcher),'launcher_sha256':sha(launcher),'wrapper':str(wrapper),'wrapper_sha256':sha(wrapper),'restart_required':True,'full_mcp_complete':False}
(base/'installation-verified.json').write_text(json.dumps(verified,indent=2)+'\n');print(json.dumps(verified),flush=True)
