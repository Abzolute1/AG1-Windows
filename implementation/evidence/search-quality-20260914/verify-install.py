import hashlib,json,stat,subprocess
from pathlib import Path
out=Path('implementation/evidence/search-quality-20260914')
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
protected={'/home/alex/.ag1/agent/llamacpp-model-profiles.json':'aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0','.local/p13-runtime-capture-20260907/configuration.json':'b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba','product/target/release/ag1-process-host':'e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2'}
for path,expected in protected.items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==expected,path
(out/'protected-hashes.json').write_text(json.dumps(protected,indent=2)+'\n')
for path,expected in json.loads((out/'source-after.json').read_text()).items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==expected,path
cli=subprocess.run([str(release/'bin/ag1-cli'),'--version'],capture_output=True,text=True,check=True).stdout.strip()
report={'status':'passed','release':str(release),'filesVerified':len(entries),'archive':package['archive'],'receipt':receipt,'cli':cli,'protectedFilesUnchanged':True,'taskSourceHashesMatchRetainedFinal':True}
(out/'systemwide-verification.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report,indent=2))
