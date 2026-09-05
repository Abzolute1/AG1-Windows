"""Stage the Linux development bridge; reuse the verified external SDK/runtime tree."""
import json,shlex,shutil
from pathlib import Path
from common import ROOT,sha256,write_json,GateError

def main():
 target=ROOT/'.local/p04-package';binary=ROOT/'product/target/release/ag1-desktop'
 if b'AG1_VISUAL_SOCKET' in binary.read_bytes():raise GateError('Refusing to package the diagnostic visual broker')
 (target/'bin').mkdir(parents=True,exist_ok=True)
 for name in ['ag1-desktop','ag1-process-host']:
  shutil.copy2(ROOT/'product/target/release'/name,target/'bin'/name)
 launcher=target/'run-ag1';launcher.write_text('#!/bin/sh\nset -eu\nexport AG1_INSTALL_ROOT='+shlex.quote(str(ROOT))+'\nexport GDK_BACKEND=x11\nexport WEBKIT_DISABLE_DMABUF_RENDERER=1\nexec "$(dirname "$0")/bin/ag1-desktop" "$@"\n');launcher.chmod(0o755)
 shutil.copytree(ROOT/'product/licenses',target/'licenses',dirs_exist_ok=True)
 artifacts=[{'path':str((target/'bin'/name).relative_to(ROOT)),'sha256':sha256(target/'bin'/name),'bytes':(target/'bin'/name).stat().st_size} for name in ['ag1-desktop','ag1-process-host']]
 manifest={'kind':'Linux development bridge staging; not a standalone installer','launcher':str(launcher.relative_to(ROOT)),'binaries':artifacts,'visual_broker':False,'external_dependencies':['Verified reference/ag1-bridge and its existing dependency tree','Pinned Node 24.14.0 in .local/toolchains','Private runtime manifest derived from P01','Preserved tuned llama.cpp binaries, shared libraries, profiles and model weights','System GTK3/WebKitGTK 4.1'],'limitations':'No SDK/runtime/weight redistribution. The original runtime tree remains in place; no live model was launched by packaging.'}
 write_json(target/'manifest.json',manifest);write_json(ROOT/'implementation/evidence/P04/package.json',manifest);print(json.dumps(manifest))
if __name__=='__main__':main()
