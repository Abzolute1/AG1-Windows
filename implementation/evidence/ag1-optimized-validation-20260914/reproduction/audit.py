import json,hashlib,subprocess,os
from pathlib import Path
e=Path("/home/alex/Projects/AG1/implementation/evidence/ag1-optimized-validation-20260914")
past=Path("/home/alex/Projects/AG1/implementation/evidence/llama-performance-20260914")
before=json.loads((e/"protected-before.json").read_text())
after={}
for name,old in before.items():
 p=Path(name)
 with p.open("rb") as f:h=hashlib.file_digest(f,"sha256").hexdigest()
 after[name]={"sha256":h,"size":p.stat().st_size,"unchanged":h==old["sha256"] and p.stat().st_size==old["size"]}
(e/"protected-after.json").write_text(json.dumps(after,indent=2))
print("Protected runtime/configuration files:",len(after),"unchanged:",all(v["unchanged"] for v in after.values()))

materials=[]
for old in json.loads((past/"model-materials-before.json").read_text()):
 p=Path(old["path"])
 with p.open("rb") as f:h=hashlib.file_digest(f,"sha256").hexdigest()
 materials.append({"path":str(p),"sha256":h,"size":p.stat().st_size,"unchanged":h==old["sha256"] and p.stat().st_size==old["size"]})
(e/"model-materials-after.json").write_text(json.dumps(materials,indent=2))
git_before=json.loads((past/"original-git.json").read_text())
def git(root,*args):return subprocess.check_output(["git","-C",root,*args],text=True)
root=git_before["root"]
git_after={"root":root,"commit":git(root,"rev-parse","HEAD").strip(),"status":git(root,"status","--porcelain"),"remotes":git(root,"remote","-v")}
git_after["unchanged"]=all(git_after[k]==git_before[k] for k in ["commit","status","remotes"])
(e/"original-git-after.json").write_text(json.dumps(git_after,indent=2))
print("Model/projector unchanged:",all(m["unchanged"] for m in materials),"original source unchanged:",git_after["unchanged"])
