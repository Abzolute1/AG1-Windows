import datetime,hashlib,json,stat,subprocess
from pathlib import Path
root=Path('/home/alex/Projects/AG1');out=root/'implementation/evidence/overnight-final-followup-20260913'
report=json.loads((out/'package.json').read_text());release=Path('/opt/ag1/current').resolve(strict=True)
manifest=json.loads((release/'package-manifest.json').read_text())
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert sha(release/'package-manifest.json')==report['manifest']['sha256']
files=manifest['files'];assert {p.relative_to(release).as_posix() for p in release.rglob('*') if p.is_file()}=={r['path'] for r in files}|{'package-manifest.json'}
for row in files:
 p=release/row['path'];assert p.stat().st_size==row['bytes'] and sha(p)==row['sha256'] and stat.S_IMODE(p.stat().st_mode)==row['mode'],row['path']
assert Path('/usr/local/bin/ag1-desktop').read_text()=='#!/bin/sh\nexec /opt/ag1/current/run-ag1 "$@"\n'
assert 'Exec=/usr/local/bin/ag1-desktop\n' in Path('/usr/share/applications/ag1.desktop').read_text()
for size in [16,24,32,48,64,128,256,512,1024]:assert sha(Path(f'/usr/share/icons/hicolor/{size}x{size}/apps/ag1.png'))==sha(release/f'icons/{size}x{size}.png')
cli=subprocess.run([str(release/'bin/ag1-cli'),'--version'],capture_output=True,text=True,check=True).stdout.strip()
protected={Path('/home/alex/.ag1/agent/llamacpp-model-profiles.json'):'aa02688c4d39ef73234f0ec381eed22dbe02463881626423049b75b5a12ad0d0',root/'product/target/release/ag1-process-host':'e1c7ca1711b2b7834eea1c99a55fe958cb4cfb880ed3c6909b06232c36bc1da2',root/'.local/p13-runtime-capture-20260907/configuration.json':'b6ea6f521e34c0278dd5d3c097ca2c83036eeebd52cb49305c4623734f3a9fba'}
for p,expected in protected.items():assert sha(p)==expected,str(p)
receipt={'status':'installed-and-manifest-verified','timestamp':datetime.datetime.now(datetime.UTC).isoformat(),'release':str(release),'verified_files':len(files),'archive':report['archive'],'input_sha256':report['input_sha256'],'system_launcher_and_icons_verified':True,'cli_smoke':cli,'protected_materials_unchanged':True,'active_app_restarted':False,'installer_model_load':False,'short_live_validation':'see live-review.json','eight_hour_run':'not started'}
(out/'installation-systemwide.json').write_text(json.dumps(receipt,indent=2)+'\n');print(json.dumps(receipt,indent=2))
