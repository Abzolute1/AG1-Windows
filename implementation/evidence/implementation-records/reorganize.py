"""One-time documentation relocation; preserve originals and machine-bound inputs."""
from pathlib import Path
import os,json,hashlib,re,tarfile,subprocess,datetime
ROOT=Path.cwd();IMPL=ROOT/'implementation';OUT=IMPL/'evidence/implementation-records'
assert not (OUT/'original-documents.tar.gz').exists()
sha=lambda b:hashlib.sha256(b).hexdigest()
originals={p:p.read_bytes() for p in sorted(IMPL.glob('*.md'))}
def category(name):
 if name=='README.md':return 'overview'
 if name.startswith('HANDOFF-OVERNIGHT') or name.startswith('HANDOFF-TEST'):return 'overnight'
 if name.startswith('MCP-'):return 'mcp'
 if 'GITHUB' in name:return 'github'
 if name in ['P10-HANDOFF.md','P11-HANDOFF.md','P12-HANDOFF.md','P13-HANDOFF.md','P13-PROGRESS.md','P14-HANDOFF.md','P14-RESUME-HANDOFF.md','P14-PROGRESS.md']:return 'phases'
 return 'desktop'
mapping={p:OUT/category(p.name)/('README-PREVIOUS.md' if p.name=='README.md' else p.name) for p in originals}
def normalize(path):return Path(os.path.abspath(path))
def rebase(text,old,new):
 def target(raw):
  if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:',raw) or raw.startswith('#'):return raw
  match=re.match(r'([^?#]*)(.*)',raw);path,suffix=match.groups()
  if not path:return raw
  dest=normalize(Path(path) if path.startswith('/') else old.parent/path)
  dest=mapping.get(dest,dest)
  # The implementation README is a live entry point, even after archiving its prior contents.
  if dest==mapping.get(IMPL/'README.md'):dest=IMPL/'README.md'
  return os.path.relpath(dest,new.parent)+suffix
 text=re.sub(r'(?<=\]\()([^\s)]+)',lambda m:target(m[0]),text)
 text=re.sub(r'^(\[[^\]\n]+\]:\s*)(\S+)',lambda m:m[1]+target(m[2]),text,flags=re.M)
 return text
paths=[Path(s) for s in subprocess.check_output(['rg','--files','-g','*.md','-g','!implementation/evidence/**','-g','!vendor/**','-g','!reference/**'],text=True).splitlines()]
external={}
for relative in paths:
 p=ROOT/relative
 if p in originals:continue
 before=p.read_text();after=rebase(before,p,p)
 # Refresh literal repo-relative documentation references in active documentation too.
 for old,new in mapping.items():
  if old.name!='README.md':after=after.replace(str(old.relative_to(ROOT)),str(new.relative_to(ROOT)))
 if after!=before:external[p]=(before.encode(),after)
# Always preserve the task index before consolidating its historical preamble.
task_index=ROOT/'tasks/README.md'
external.setdefault(task_index,(task_index.read_bytes(),task_index.read_text()))
with tarfile.open(OUT/'original-documents.tar.gz','w:gz') as tar:
 for p in list(originals)+list(external):tar.add(p,arcname=str(p.relative_to(ROOT)),recursive=False)
rows=[]
for old,raw in originals.items():
 new=mapping[old];new.parent.mkdir(parents=True,exist_ok=True)
 banner='> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status]('+os.path.relpath(IMPL/'README.md',new.parent)+'). Original bytes and old paths are preserved in the [relocation manifest]('+os.path.relpath(OUT/'manifest.json',new.parent)+') and recovery archive.\n\n'
 text=rebase(raw.decode(),old,new);lines=text.splitlines(keepends=True)
 if lines and lines[0].startswith('# '):text=lines[0]+'\n'+banner+''.join(lines[1:]).lstrip('\n')
 else:text=banner+text
 new.write_text(text)
 rows.append({'old_path':str(old.relative_to(ROOT)),'new_path':str(new.relative_to(ROOT)),'original_sha256':sha(raw),'original_bytes':len(raw),'category':category(old.name),'archived_copy_sha256':sha(new.read_bytes())})
for p,(before,after) in external.items():p.write_text(after)
# Preserve exact bytes at the two paths consumed by executable release tooling.
for name in ['P12-HANDOFF.md','P13-HANDOFF.md']:
 old=IMPL/name;dest=OUT/'compatibility'/name;dest.parent.mkdir(exist_ok=True);dest.write_bytes(originals[old]);old.unlink();old.symlink_to(os.path.relpath(dest,old.parent))
for old in originals:
 if old.name not in ['README.md','P12-HANDOFF.md','P13-HANDOFF.md']:old.unlink()
manifest={'timestamp':datetime.datetime.now(datetime.UTC).isoformat(),'purpose':'Archive historical implementation notes and preserve exact original evidence; active work remains in tasks.','original_archive':'implementation/evidence/implementation-records/original-documents.tar.gz','archive_sha256':sha((OUT/'original-documents.tar.gz').read_bytes()),'records':rows,'external_documents_before':[{'path':str(p.relative_to(ROOT)),'sha256':sha(before)} for p,(before,after) in external.items()],'compatibility_paths':['implementation/P12-HANDOFF.md','implementation/P13-HANDOFF.md'],'historical_reference_policy':'Existing evidence files and receipts were not rewritten. Old paths in them map through records to readable archived copies; exact original bytes are in original-documents.tar.gz under old_path. P12/P13 retain exact-byte compatibility paths for executable gates.'}
(OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
# Topic index; only the index denotes current versus historical meaning.
text='# Archived implementation records\n\nThese are historical checkpoints, not a work queue. Start with [current implementation status](../../README.md) and [remaining tasks](../../../../tasks/README.md). Read an individual record for its implementation details, failures and evidence.\n\nOriginal bytes of all 76 former top-level Markdown files, plus changed incoming-link documents, are in [original-documents.tar.gz](original-documents.tar.gz). The [manifest](manifest.json) maps every old path and hash to its readable archived copy. Existing evidence and receipts retain their original content and paths; use that map when following a relocated historical reference. Two [compatibility originals](compatibility/) preserve P12/P13 inputs used by release tooling.\n\n'
labels={'overnight':'Overnight investigation, fixes and completed scenario handoff','mcp':'MCP implementation and acceptance','github':'GitHub implementation and acceptance','desktop':'Desktop, model interface and artifact/review history','phases':'Phase entry, progress and release custody','overview':'Previous implementation overview'}
for group,title in labels.items():
 text+=f'## {title}\n\n'
 for row in rows:
  if row['category']==group:
   new=ROOT/row['new_path'];title=originals[ROOT/row['old_path']].decode().splitlines()[0].lstrip('# ')
   text+=f'- [{title}]({os.path.relpath(new,OUT)})\n'
 text+='\n'
(OUT/'README.md').write_text(text)
print(json.dumps({'archived_top_level_documents':len(rows),'incoming_documents_updated':len(external),'compatibility_links':2},indent=2))
