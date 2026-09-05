"""Retain pinned SQLite source notice and the omitted WASM VFS repository license."""
import json,subprocess,urllib.request
from pathlib import Path
from common import ROOT,sha256,write_json,GateError

def main():
 metadata=json.loads(subprocess.check_output(['cargo','metadata','--manifest-path',str(ROOT/'product/Cargo.toml'),'--locked','--offline','--format-version','1']))
 package=next(p for p in metadata['packages'] if p['name']=='rsqlite-vfs')
 source=Path(package['manifest_path']).parent
 commit=json.loads((source/'.cargo_vcs_info.json').read_text())['git']['sha1']
 # Published workspace README identifies sqlite-wasm-rs; verify its member at the exact VCS pin.
 base=f'https://raw.githubusercontent.com/Spxg/sqlite-wasm-rs/{commit}/'
 target=ROOT/'product/licenses'/f"rsqlite-vfs-{package['version']}";target.mkdir(exist_ok=True)
 license_path=target/'LICENSE';proof=target/'SOURCE-Cargo.toml'
 for filename,out in [('LICENSE',license_path),('crates/rsqlite-vfs/Cargo.toml',proof)]:
  if not out.exists():
   with urllib.request.urlopen(base+filename,timeout=30) as response:out.write_bytes(response.read())
 if 'MIT License' not in license_path.read_text() or 'name = "rsqlite-vfs"' not in proof.read_text():raise GateError('Unexpected pinned VFS license/source')
 path=ROOT/'product/licenses/crate-license-fallbacks.json';fallback=json.loads(path.read_text())
 fallback[f"rsqlite-vfs@{package['version']}"]=[{'path':str(out.relative_to(ROOT)),'sha256':sha256(out),'source':base+name,'reason':'Published crate omitted workspace license; exact VCS member identity retained.'} for name,out in [('LICENSE',license_path),('crates/rsqlite-vfs/Cargo.toml',proof)]]
 write_json(path,fallback)
 sqlite=next(p for p in metadata['packages'] if p['name']=='libsqlite3-sys');source=Path(sqlite['manifest_path']).parent/'sqlite3/sqlite3.c'
 target=ROOT/'product/licenses'/f"libsqlite3-sys-{sqlite['version']}";target.mkdir(exist_ok=True)
 notice=target/'SQLITE-SOURCE-NOTICE';notice.write_text('\n'.join(source.read_text().splitlines()[:45])+'\n')
 write_json(ROOT/'implementation/evidence/P05/storage-dependencies.json',{'rusqlite':'0.40.2','libsqlite3_sys':sqlite['version'],'sqlite_amalgamation_sha256':sha256(source),'sqlite_notice':{'path':str(notice.relative_to(ROOT)),'sha256':sha256(notice)},'vfs_license':fallback[f"rsqlite-vfs@{package['version']}"],'scope':'Bundled SQLite is storage only; no llama.cpp runtime changes. WASM target dependencies are recorded but not validated as a storage platform.'})
 print('Retained SQLite source notice and exact VFS license')
if __name__=='__main__':main()
