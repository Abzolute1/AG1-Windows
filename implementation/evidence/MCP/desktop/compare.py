from pathlib import Path
import json,re
out=Path('/home/alex/Projects/AG1/.local/mcp-b6')
def failures(p):
 s=p.read_text();rows={}
 for name,body in re.findall(r'---- (.*?) stdout ----\n\n(.*?)(?=\n\n---- |\n\n\nfailures:)',s,re.S):
  body=re.sub(r'\(\d+\) panicked','(PID) panicked',body.strip())
  body=re.sub(r'\n?note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace','',body)
  rows[name]=body
 m=re.search(r'test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed',s)
 assert m and len(rows)==int(m[2]),p
 return rows,{'passed':int(m[1]),'failed':int(m[2])}
for suite in ['session','core']:
 b,bc=failures(out/f'baseline-{suite}.log');c,cc=failures(out/f'final-{suite}.log')
 result={'baseline_results':bc,'candidate_results':cc,'same_failure_names':b.keys()==c.keys(),'identical_assertion_bodies':b==c,'normalization':'Nondeterministic thread IDs and once-per-process backtrace-help line only','baseline_only':sorted(b.keys()-c.keys()),'candidate_only':sorted(c.keys()-b.keys()),'failures':{k:{'baseline':b.get(k),'candidate':c.get(k),'identical':b.get(k)==c.get(k)} for k in sorted(b.keys()|c.keys())}}
 (out/f'{suite}-comparison.json').write_text(json.dumps(result,indent=2)+'\n')
 print(suite,bc,cc,'equal assertions',b==c,'baseline only',result['baseline_only'],'candidate only',result['candidate_only'])
 assert b==c and bc==cc,(suite,'Review retained differences; do not weaken assertion')

def ui(p):
 s=re.sub(r'\x1b\[[0-9;]*m','',p.read_text());result={}
 for line in s.splitlines():
  if line.strip().startswith('FAIL '):
   name=line.strip()[5:];tail=s[s.index(line)+len(line):]
   reason=next((x.strip() for x in tail.splitlines() if x.strip()),'')
   result[name]=reason
 m=re.search(r'Tests\s+(\d+) failed \| (\d+) passed \((\d+)\)',s)
 assert m
 return result,{'failed':int(m[1]),'passed':int(m[2]),'total':int(m[3])}
b,bc=ui(out/'baseline-ui.log');c,cc=ui(out/'final-ui.log')
result={'baseline_results':bc,'candidate_results':cc,'same_failure_names_and_reasons':b==c,'failures':c,'limitation':'Full DOM diagnostic bodies differ where the MCP navigation was intentionally added; compare exact failure names and primary assertion messages. Raw complete logs retained.'}
(out/'ui-comparison.json').write_text(json.dumps(result,indent=2)+'\n')
print('ui',bc,cc,'same reasons',b==c)
assert b==c and len(c)==2 and cc['passed']==bc['passed']+5
