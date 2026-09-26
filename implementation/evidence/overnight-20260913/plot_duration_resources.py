"""Create final resource figures from retained completed-run receipts only."""
import collections,hashlib,json,sys
from pathlib import Path

def main():
    import matplotlib
    matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    run=Path(sys.argv[1]).resolve()
    result_path=run/'duration-result.json'
    result=json.loads(result_path.read_text())
    start=result['start_monotonic_ns'];end=result['end_monotonic_ns']
    samples=result['samples'];groups=collections.defaultdict(list);models=collections.defaultdict(list)
    for s in samples:
        if not s.get('host_present'):continue
        h=s['heap_observation'];host=next(p for p in s['processes'] if p['pid']==h['pid'])
        key=(host['pid'],host['start_ticks'],host['executable_device'],host['executable_inode'])
        groups[key].append((s,host))
        for p in s['processes']:
            if Path(p['executable']).name=='llama-server':
                identity=(p['pid'],p['start_ticks'],p['executable_device'],p['executable_inode'])
                models[identity].append(((s['monotonic_ns']-start)/3.6e12,p['pss_bytes']/2**30))
    plt.rcParams.update({'font.family':'DejaVu Sans','font.size':10,'axes.spines.top':False,'axes.spines.right':False})
    fig,axes=plt.subplots(4,1,figsize=(12,12),sharex=True,layout='constrained')
    palette=plt.get_cmap('tab10')
    for i,(key,rows) in enumerate(groups.items()):
        xs=[(s['monotonic_ns']-start)/3.6e12 for s,p in rows];color=palette(i%10)
        axes[0].plot(xs,[s['heap_bytes']/2**20 for s,p in rows],color=color,label=f'Host {key[0]}',linewidth=1.4,marker='.' if len(rows)<5 else None)
        axes[1].plot(xs,[p['pss_bytes']/2**20 for s,p in rows],color=color,linewidth=1.3)
        axes[1].plot(xs,[s['heap_observation']['allocator_reserved_bytes']/2**20 for s,p in rows],color=color,linestyle=':',linewidth=1.1)
        axes[0].axvspan(xs[0],min(xs[-1],xs[0]+result['warmup_seconds']/3600),color=color,alpha=.06)
    for i,(key,rows) in enumerate(models.items()):
        axes[2].plot([r[0] for r in rows],[r[1] for r in rows],label=f'Model {key[0]}',linewidth=1.3,color=palette(i%10))
    axes[3].plot([(s['monotonic_ns']-start)/3.6e12 for s in samples],[s['persistent_evidence_bytes']/2**30 for s in samples],color='#334e68',linewidth=1.8)
    names={'corrupt-state-recovery':'Corrupt-state recovery','model-crash':'Model crash','host-crash':'Host crash'}
    for a in result['activities']:
        if a['kind'] not in names:continue
        x=(a['started_monotonic_ns']-start)/3.6e12
        for ax in axes:ax.axvline(x,color='#a33c32',linestyle='--',alpha=.5,linewidth=.9)
        axes[0].annotate(names[a['kind']],xy=(x,1),xycoords=('data','axes fraction'),xytext=(4,-5),textcoords='offset points',va='top',fontsize=8,color='#862e27')
    axes[0].set_ylabel('Host heap estimate (MiB)');axes[0].legend(loc='upper left',bbox_to_anchor=(1.005,1),fontsize=8)
    axes[1].set_ylabel('Host memory (MiB)');axes[1].set_title('Solid: host PSS · dotted: allocator reserved',loc='left',fontsize=10)
    axes[2].set_ylabel('Model PSS (GiB)');axes[2].legend(loc='upper left',bbox_to_anchor=(1.005,1),fontsize=8)
    axes[3].set_ylabel('Native state on disk (GiB)');axes[3].set_xlabel('Elapsed workload hours')
    for ax in axes:ax.grid(axis='y',alpha=.2);ax.set_ylim(bottom=0)
    hours=(end-start)/3.6e12
    axes[3].set_xlim(0,hours)
    fig.suptitle(f'AG1 / Qwen35B long-horizon investigation — {hours:.2f} hours\nDuration controller status: {result["status"]}',fontsize=15,fontweight='bold')
    fig.supxlabel('Each process lifetime is separate; missing hosts are not zero-memory samples. Shading marks host warmup.\nPSS is CPU process memory, not GPU VRAM. Heap is the glibc in-use-or-retained estimate. Disk excludes CLI test logs.',fontsize=9)
    dest=Path(__file__).resolve().parent
    png=dest/f'{run.name}-resources.png';pdf=dest/f'{run.name}-resources.pdf'
    if png.exists() or pdf.exists():raise RuntimeError('Final figures already exist; preserve evidence')
    fig.savefig(png,dpi=160);fig.savefig(pdf);plt.close(fig)
    receipt={'source':str(result_path),'source_sha256':hashlib.sha256(result_path.read_bytes()).hexdigest(),'elapsed_hours':hours,'controller_status':result['status'],'host_lifetimes':len(groups),'model_lifetimes':len(models),'samples':len(samples),'figures':[{'path':str(p),'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}for p in [png,pdf]],'release_acceptance':False}
    with (dest/f'{run.name}-resources.json').open('x') as f:json.dump(receipt,f,indent=2);f.write('\n')
    print(json.dumps(receipt,indent=2))
if __name__=='__main__':main()
