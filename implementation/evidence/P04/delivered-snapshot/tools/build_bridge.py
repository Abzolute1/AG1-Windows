#!/usr/bin/env python3
"""Reproduce P04 patches on a separate, pinned reference; never modify the oracle."""
from pathlib import Path
import hashlib,json,shutil,subprocess
from common import ROOT,REFERENCE,verify_original,sha256,GateError

def main():
 verify_original();target=ROOT/'reference/ag1-bridge';patches=ROOT/'product/compat/agent-worker/patches'
 if not target.exists():
  shutil.copytree(REFERENCE,target,symlinks=True,ignore=shutil.ignore_patterns('node_modules','.git'))
  (target/'node_modules').mkdir()
  for p in (REFERENCE/'node_modules').iterdir():
   if p.name!='@ag1':(target/'node_modules'/p.name).symlink_to(p.resolve(),target_is_directory=p.is_dir())
  (target/'node_modules/@ag1').mkdir()
  for p in (REFERENCE/'node_modules/@ag1').iterdir():(target/'node_modules/@ag1'/p.name).symlink_to(target/p.resolve().relative_to(REFERENCE),target_is_directory=True)
 for row in json.loads((patches/'manifest.json').read_text()):
  if sha256(REFERENCE/row['path'])!=row['original_sha256']:raise GateError('Patch base drift')
  existing=sha256(target/row['path'])
  if existing==row['patched_sha256']:continue
  if existing!=row['original_sha256']:raise GateError('Unreviewed bridge source edit: '+row['path'])
  subprocess.run(['patch','--batch','--fuzz=0','-p1','-i',str(patches/row['patch'])],cwd=target,check=True)
  if sha256(target/row['path'])!=row['patched_sha256']:raise GateError('Patched output drift')
 for file in (patches/'additions').glob('*.ts'):
  package='agent/src' if file.stem=='bridge-evidence' else 'coding-agent/src/core'
  shutil.copyfile(file,target/'packages'/package/file.name)
 for package in ['agent','coding-agent']:
  subprocess.run([str(target/'node_modules/.bin/tsgo'),'-p',str(target/'packages'/package/'tsconfig.build.json')],cwd=target,check=True)
 print('Separate bridge copy verified and compiled; original reference unchanged.')
if __name__=='__main__':main()
