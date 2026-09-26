from pathlib import Path
import json,hashlib,tarfile,os,re,ast,io
ROOT=Path.cwd();OUT=ROOT/'implementation/evidence/implementation-records';IMPL=ROOT/'implementation';manifest=json.loads((OUT/'manifest.json').read_text());sha=lambda b:hashlib.sha256(b).hexdigest()
# Include the original chat-task wording in the same recovery archive.
with tarfile.open(OUT/'original-documents.tar.gz','r:gz') as tar:original={m.name:(m,tar.extractfile(m).read()) for m in tar.getmembers() if m.isfile()}
if 'tasks/03-CHAT-CLEANUP.md' not in original:
 p=ROOT/'tasks/03-CHAT-CLEANUP.md';raw=p.read_text().replace('copy fidelity and long-history behavior','copy fidelity and virtualized long-history behavior').replace(' Existing containment retains loaded DOM/React state; it is not DOM virtualization.','').encode();info=tarfile.TarInfo('tasks/03-CHAT-CLEANUP.md');info.size=len(raw);info.mode=0o644;original[info.name]=(info,raw)
 temp=OUT/'original-documents.tar.gz.new'
 with tarfile.open(temp,'w:gz') as tar:
  for info,raw in original.values():tar.addfile(info,io.BytesIO(raw))
 temp.replace(OUT/'original-documents.tar.gz');manifest['external_documents_before'].append({'path':'tasks/03-CHAT-CLEANUP.md','sha256':sha(raw)})
# Render archival banners after relative-link rebasing. Originals remain exact in the archive.
mapping={ROOT/row['old_path']:ROOT/row['new_path'] for row in manifest['records']}
module=ast.parse((OUT/'reorganize.py').read_text());functions=[n for n in module.body if isinstance(n,ast.FunctionDef) and n.name in ['normalize','rebase']];ns=dict(Path=Path,os=os,re=re,mapping=mapping,IMPL=IMPL);exec(compile(ast.Module(body=functions,type_ignores=[]),'<link relocation>','exec'),ns)
for row in manifest['records']:
 old=ROOT/row['old_path'];new=ROOT/row['new_path'];raw=original[row['old_path']][1];assert sha(raw)==row['original_sha256'];text=ns['rebase'](raw.decode(),old,new)
 banner='> Historical evidence, archived 2026-09-13. Status, commands, permissions and paths below describe their original checkpoint. For current work start with [implementation status]('+os.path.relpath(IMPL/'README.md',new.parent)+'). Original bytes and old paths are preserved in the [relocation manifest]('+os.path.relpath(OUT/'manifest.json',new.parent)+') and recovery archive.\n\n'
 lines=text.splitlines(keepends=True);text=lines[0]+'\n'+banner+''.join(lines[1:]).lstrip('\n') if lines and lines[0].startswith('# ') else banner+text
 new.write_text(text);row['archived_copy_sha256']=sha(new.read_bytes())
p=OUT/'README.md';s=p.read_text().replace('../../../../tasks/README.md','../../../tasks/README.md');p.write_text(s)
manifest['archive_sha256']=sha((OUT/'original-documents.tar.gz').read_bytes());(OUT/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
# One bounded integrity/link review, without builds or application activity.
for row in manifest['external_documents_before']:assert sha(original[row['path']][1])==row['sha256'],row['path']
for name in ['P12-HANDOFF.md','P13-HANDOFF.md']:
 p=IMPL/name;assert p.is_symlink() and p.read_bytes()==original['implementation/'+name][1]
assert sorted(p.name for p in IMPL.iterdir())==['AGENTS.md','P12-HANDOFF.md','P13-HANDOFF.md','README.md','evidence']
def links(text,parent):
 out=[]
 for match in re.finditer(r'(?<=\]\()([^\s)]+)',text):
  raw=match[0]
  if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*:',raw) or raw.startswith('#'):continue
  path=raw.split('#')[0].split('?')[0]
  if path:out.append((raw,Path(os.path.abspath(parent/path))))
 return out
new_docs=[IMPL/'README.md',IMPL/'AGENTS.md',IMPL/'evidence/README.md',OUT/'README.md'];new_broken=[]
for p in new_docs:
 for raw,target in links(p.read_text(),p.parent):
  if not target.exists():new_broken.append({'file':str(p.relative_to(ROOT)),'link':raw})
assert not new_broken,new_broken
introduced=[];preexisting=[]
for row in manifest['records']:
 old=ROOT/row['old_path'];new=ROOT/row['new_path'];old_links=links(original[row['old_path']][1].decode(),old.parent);new_links=links(new.read_text(),new.parent)[2:]
 assert len(old_links)==len(new_links),(old,len(old_links),len(new_links))
 for (before,t0),(after,t1) in zip(old_links,new_links):
  if t1.exists():continue
  existed=t0 in mapping or t0.exists()
  item={'file':row['new_path'],'link':after,'original_link':before}
  (introduced if existed else preexisting).append(item)
for row in manifest['external_documents_before']:
 p=ROOT/row['path'];before_links=links(original[row['path']][1].decode(),p.parent);previous_missing={t for _,t in before_links if not t.exists() and t not in mapping}
 for raw,target in links(p.read_text(),p.parent):
  if not target.exists() and target not in previous_missing:introduced.append({'file':row['path'],'link':raw})
assert not introduced,introduced
result={'status':'passed','archived_records':len(manifest['records']),'original_documents_preserved':len(original),'top_level_markdown_before':76,'top_level_regular_markdown_after':2,'compatibility_symlinks':2,'introduced_broken_file_links':introduced,'preexisting_unavailable_historical_links':preexisting,'original_hashes_verified':True,'existing_evidence_files_rewritten':False,'software_builds_or_model_runs':False}
(OUT/'verification.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k!='preexisting_unavailable_historical_links'},indent=2));print('Pre-existing unavailable historical links:',len(preexisting))
