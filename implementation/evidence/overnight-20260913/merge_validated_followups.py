"""Selective, hash-admitted merge after the real duration and preservation audit."""
import hashlib,json,os,shutil,tarfile,tempfile
from pathlib import Path
ROOT=Path('/home/alex/Projects/AG1');OUT=ROOT/'implementation/evidence/overnight-20260913';RUN=ROOT/'.local/p14-long-horizon/overnight-7'
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
result=json.loads((RUN/'duration-result.json').read_text())
elapsed_hours=(result['end_monotonic_ns']-result['start_monotonic_ns'])/3.6e12
if elapsed_hours<8 and result['status']!='failed':raise RuntimeError('Duration has neither finished eight hours nor terminated with a retained failure')
preservation=json.loads((RUN/'post-duration-preservation/result.json').read_text())
if preservation['status']!='preservation-checked':raise RuntimeError('Post-duration preservation is incomplete')
manifest=json.loads((OUT/'performance-followup-manifest.json').read_text());candidate=Path(manifest['source_root']);rows=manifest['files']
for row in rows:
 name=Path(row['path']);assert not name.is_absolute() and '..' not in name.parts
 a=ROOT/name;b=candidate/name
 if a.is_symlink() or b.is_symlink() or not a.is_file() or not b.is_file():raise RuntimeError('Expected regular source files: '+str(name))
 if digest(a)!=row['frozen_base_sha256'] or digest(b)!=row['candidate_sha256']:raise RuntimeError('Source changed since validation; review rather than overwriting: '+str(name))
backup=OUT/'pre-followup-source.tar.gz'
if backup.exists() or (OUT/'followup-merge-result.json').exists():raise RuntimeError('Merge evidence already exists; review existing state')
with tarfile.open(backup,'x:gz') as archive:
 for row in rows:archive.add(ROOT/row['path'],arcname=row['path'],recursive=False)
for row in rows:
 a=ROOT/row['path'];b=candidate/row['path']
 with tempfile.NamedTemporaryFile(dir=a.parent,prefix='.ag1-followup-',delete=False) as temp:
  temp.write(b.read_bytes());temp.flush();os.fsync(temp.fileno());name=Path(temp.name)
 try:shutil.copystat(a,name);os.replace(name,a)
 finally:name.unlink(missing_ok=True)
for row in rows:assert digest(ROOT/row['path'])==row['candidate_sha256']
receipt={'status':'merged-awaiting-broader-validation','files':rows,'backup':str(backup),'backup_sha256':digest(backup),'duration_status_unchanged':result['status'],'observed_duration_hours':elapsed_hours,'eight_hour_workload_achieved':elapsed_hours>=8,'duration_candidate_predates_followups':True,'release_acceptance':False}
with (OUT/'followup-merge-result.json').open('x') as f:json.dump(receipt,f,indent=2);f.write('\n')
print('Merged only '+str(len(rows))+' validated files; broader completion checks remain.')
