#!/usr/bin/env python3
"""Complete license material omitted from locked crates; retain exact source notices."""
import json,urllib.request,hashlib,shutil
from pathlib import Path
from common import ROOT,sha256,write_json,GateError

def main():
 rows=json.loads((ROOT/'implementation/evidence/P04/missing-crate-license-files.json').read_text());result={}
 for row in rows:
  name,version=row['name'],row['version'];directory=Path(row['dir']);repo=row['repository'];vcs=row['vcs'];commit=vcs.get('git',{}).get('sha1')
  if name in ['r-efi','ts-rs','ts-rs-macros','plist']:continue
  target=ROOT/'product/licenses'/f'{name}-{version}';target.mkdir(exist_ok=True);files=[]
  if not repo and name=='libappindicator-sys':repo='https://github.com/tauri-apps/libappindicator-rs'
  def keep(out,source,reason):files.append({'path':str(out.relative_to(ROOT)),'sha256':sha256(out),'source':source,'reason':reason})
  if name=='selectors':
   out=target/'LICENSE-MPL-2.0';url='https://www.mozilla.org/media/MPL/2.0/index.txt'
   if not out.exists():
    with urllib.request.urlopen(url,timeout=30) as response:out.write_bytes(response.read())
   if 'Mozilla Public License Version 2.0' not in out.read_text():raise GateError('Unexpected MPL text')
   keep(out,url,'Published source explicitly names MPL 2.0; full standard license retained.')
   source=directory/'lib.rs';out=target/'SOURCE-NOTICE';out.write_text('\n'.join(source.read_text().splitlines()[:5])+'\n');keep(out,str(source),'Unmodified source copyright and license notice.')
   archive=next((Path.home()/'.cargo/registry/cache').glob(f'*/{name}-{version}.crate'));out=target/archive.name;shutil.copyfile(archive,out);keep(out,str(archive),'Exact unmodified published source supplied for MPL obligations.')
  elif name.startswith('winapi-'):
   source=directory/'src/lib.rs';out=target/'SOURCE-NOTICE';out.write_text('\n'.join(source.read_text().splitlines()[:6])+'\n');keep(out,str(source),'Published source offers Apache 2.0 or MIT; choose Apache 2.0.')
   out=target/'LICENSE-APACHE';source=next(directory.parent.glob('winapi-0.3.*/LICENSE-APACHE'));shutil.copyfile(source,out);keep(out,str(source),'Full Apache 2.0 license named by the published source notice; GNU Windows target only.')
  else:
   if not commit:raise GateError('No published VCS identity: '+name)
   base=f"https://raw.githubusercontent.com/{repo.split('github.com/')[1].strip('/').removesuffix('.git')}/{commit}/"
   candidates=[f.name for f in target.iterdir() if f.is_file() and f.name.lower().startswith(('license','licence','copying','copyright'))]
   if not candidates:candidates=['LICENSE','LICENSE-MIT','LICENSE-APACHE','LICENSE.md','LICENSE.txt']
   for source in candidates:
    out=target/source;url=base+source
    if not out.exists():
     try:
      with urllib.request.urlopen(url,timeout=20) as response:out.write_bytes(response.read())
     except urllib.error.HTTPError as e:
      if e.code==404:continue
      raise
    keep(out,url,'License omitted from crate; exact published VCS revision retained.')
   if name.startswith(('objc2','block2','dispatch2')):
    source=directory/'Cargo.toml';out=target/'PUBLISHED-METADATA.toml';shutil.copyfile(source,out);keep(out,str(source),'Retain authors and license metadata; Apple SDK-derived bindings remain a macOS platform/distribution review gate.')
  if not files:raise GateError('No license material: '+name)
  result[f'{name}@{version}']=files
  write_json(ROOT/'product/licenses/crate-license-fallbacks.json',result)
 print('Retained license material:',len(result))
if __name__=='__main__':main()
