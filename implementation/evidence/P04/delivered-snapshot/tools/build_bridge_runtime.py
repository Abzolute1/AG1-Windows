#!/usr/bin/env python3
"""Create the private bridge manifest from verified P01 hashes; no model actions."""
import json
from pathlib import Path
from common import ROOT,GateError,sha256
from runtime_manifest import signature

def main():
 lock=json.loads((ROOT/'planning/evidence/llama-runtime-lock.json').read_text())
 oracle=json.loads((ROOT/'implementation/evidence/P01/runtime-manifest.json').read_text())
 cache=json.loads((ROOT/'.local/runtime/hash-cache.json').read_text())
 if sha256(Path(lock['profile_file']))!=lock['profile_file_sha256']:raise GateError('Original profile drift; refusing to replace it')
 material=[]
 for path,item in sorted(cache.items()):
  if signature(Path(path))!=item['signature']:raise GateError('Runtime material identity changed; revalidation required: '+path)
  material.append({'path':path,'sha256':item['sha256'],'signature':list(map(str,item['signature']))})
 models={}
 for model,row in lock['models'].items():
  weights=sorted(w['path'] for w in oracle['weights'] if w['model']==model)
  if not weights:raise GateError('Missing model weight evidence: '+model)
  models[model]={'profile':row['profile'],'path':weights[0]}
 output={'models':models,'material':material,'profile_file':lock['profile_file'],'profile_sha256':lock['profile_file_sha256']}
 target=ROOT/'.local/bridge-runtime.json';target.write_text(json.dumps(output,indent=2));target.chmod(0o600)
 print(f'Preserved {len(models)} profiles and {len(material)} exact material identities; no runtime launched or changed.')
if __name__=='__main__':main()
