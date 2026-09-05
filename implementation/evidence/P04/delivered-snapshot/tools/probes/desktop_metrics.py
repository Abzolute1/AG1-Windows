"""Read only the recorded desktop and its descendants; include every worker process."""
import argparse,json,os,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
def process_table():
 rows={}
 for p in Path('/proc').iterdir():
  if not p.name.isdigit():continue
  try:
   s=(p/'stat').read_text().rsplit(') ',1)[1].split();rows[int(p.name)]={'pid':int(p.name),'ppid':int(s[1]),'start':s[19],'ticks':int(s[11])+int(s[12]),'rss_kib':int(s[21])*os.sysconf('SC_PAGE_SIZE')//1024,'name':(p/'comm').read_text().strip()}
  except (OSError,ValueError,IndexError):pass
 return rows
def sample(record):
 rows=process_table();pid=record['pid'];assert pid in rows and rows[pid]['start']==record['start'],'Owned process exited or PID reused'
 owned={pid}
 while True:
  more={p for p,row in rows.items() if row['ppid'] in owned}-owned
  if not more:break
  owned|=more
 result=[]
 for pid in sorted(owned):
  row=rows[pid]
  try:
   pss=next(line for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith('Pss:'));row['pss_kib']=int(pss.split()[1])
  except (OSError,StopIteration):row['pss_kib']=None
  result.append(row)
 return {'time':time.monotonic(),'processes':result,'rss_kib':sum(r['rss_kib'] for r in result),'pss_kib':sum(r['pss_kib'] or 0 for r in result)}
def main():
 parser=argparse.ArgumentParser();parser.add_argument('app',choices=['ag1','dyad']);parser.add_argument('--seconds',type=int,default=300);parser.add_argument('--label',default='idle');args=parser.parse_args()
 base=ROOT/('.local/p04-visual' if args.app=='ag1' else '.local/p04-dyad');record=json.loads((base/'process.json').read_text());samples=[sample(record)];deadline=samples[0]['time']+args.seconds
 print(f'Measuring {args.app} {args.label} for {args.seconds} seconds, including descendants.',flush=True)
 while time.monotonic()<deadline:
  time.sleep(min(5,max(0,deadline-time.monotonic())));samples.append(sample(record))
 first={(r['pid'],r['start']):r['ticks'] for r in samples[0]['processes']};last={(r['pid'],r['start']):r['ticks'] for r in samples[-1]['processes']};elapsed=samples[-1]['time']-samples[0]['time'];cpu=sum(max(0,t-first.get(k,0)) for k,t in last.items())/os.sysconf('SC_CLK_TCK')
 result={'app':args.app,'workload':args.label,'duration_seconds':elapsed,'cpu_seconds':cpu,'cpu_percent_one_core':100*cpu/elapsed,'startup':record,'samples':samples,'limitations':['RSS double counts shared pages; PSS is summed across descendants.','CPU includes live descendants; short-lived processes that exit between samples may be missed.','Warm OS caches; Linux X11 display; fixture provider; no inference.']}
 dest=ROOT/f'implementation/evidence/P04/{args.app}-{args.label}-metrics.json';dest.write_text(json.dumps(result,indent=2));print(json.dumps({'file':str(dest),'duration':elapsed,'cpu_percent_one_core':result['cpu_percent_one_core'],'final_pss_mib':samples[-1]['pss_kib']/1024}),flush=True)
if __name__=='__main__':main()
