import hashlib,json,stat,subprocess
from pathlib import Path
out=Path('implementation/evidence/model-loading-20260914')
release=Path('/opt/ag1/current').resolve();package=json.loads((out/'package-validation.json').read_text())
isolated=Path(package['installed']);manifest=(release/'package-manifest.json').read_bytes()
assert manifest==(isolated/'package-manifest.json').read_bytes()
entries=json.loads(manifest)['files']
for item in entries:
 p=release/item['path'];b=p.read_bytes()
 assert hashlib.sha256(b).hexdigest()==item['sha256'],p
 assert len(b)==item['bytes'],p
 assert stat.S_IMODE(p.stat().st_mode)==item['mode'],p
receipt=json.loads((Path('/opt/ag1/receipts')/(release.name+'.json')).read_text())
assert receipt['archive_sha256']==package['archive']['sha256'] and receipt['release']==str(release)
protected=json.loads((out/'protected-before.json').read_text())
for path,expected in protected.items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==expected,path
(out/'protected-hashes.json').write_text(json.dumps(protected,indent=2)+'\n')
for path,expected in json.loads((out/'source-after.json').read_text()).items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==expected,path
cli=subprocess.run([str(release/'bin/ag1-cli'),'--version'],capture_output=True,text=True,check=True).stdout.strip()
report={'status':'passed','release':str(release),'filesVerified':len(entries),'archive':package['archive'],'receipt':receipt,'cli':cli,'protectedFilesUnchanged':True,'taskSourceHashesMatchRetainedFinal':True}
(out/'systemwide-verification.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
