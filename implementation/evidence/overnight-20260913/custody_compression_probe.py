"""Read-only sizing experiment; does not change storage or define a new format."""
import hashlib,json,sqlite3,statistics,time,zlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3]
STORE=ROOT/'.local/p14-long-horizon/overnight-7/native/state/store'
con=sqlite3.connect('file:'+str(STORE/'storage.sqlite')+'?mode=ro',uri=True)
rows=[]
for kind in ['context-original-journal','context-original-request','runtime-request-input','runtime-request-compiled']:
    samples=con.execute("SELECT digest,bytes FROM artifacts WHERE json_extract(CAST(record AS TEXT),'$.kind')=? ORDER BY rowid DESC LIMIT 8",[kind]).fetchall()
    measurements=[]
    for digest,size in samples:
        raw=(STORE/'blobs'/digest).read_bytes()
        assert len(raw)==size and hashlib.sha256(raw).hexdigest()==digest
        for level in [1,6]:
            compress_ms=[];decompress_ms=[]
            for _ in range(3):
                t=time.perf_counter_ns();packed=zlib.compress(raw,level);compress_ms.append((time.perf_counter_ns()-t)/1e6)
                t=time.perf_counter_ns();restored=zlib.decompress(packed);decompress_ms.append((time.perf_counter_ns()-t)/1e6)
                assert restored==raw and hashlib.sha256(restored).hexdigest()==digest
            measurements.append({'sha256':digest,'raw_bytes':size,'level':level,'compressed_bytes':len(packed),'compression_ms_median':statistics.median(compress_ms),'decompression_ms_median':statistics.median(decompress_ms),'exact_reconstruction':True})
    rows.append({'kind':kind,'samples':measurements})
con.close()
result={'utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'scope':'Latest eight immutable artifacts of each large custody kind, read only','codec':'Python zlib '+zlib.ZLIB_RUNTIME_VERSION,'iterations_per_artifact_level':3,'limitations':'Small sampled sizing experiment during live workload, not a Rust storage benchmark or proposed storage format. Compression is not installed. Existing raw-blob readers, streaming hash validation, bounded reads, archive/import/recovery and garbage collection require a versioned compatible design.','groups':rows}
output=ROOT/'implementation/evidence/overnight-20260913/custody-compression-probe.json'
with output.open('x') as f:json.dump(result,f,indent=2);f.write('\n')
for row in rows:
    for level in [1,6]:
        values=[r for r in row['samples'] if r['level']==level]
        print(json.dumps({'kind':row['kind'],'level':level,'raw_bytes':sum(r['raw_bytes'] for r in values),'compressed_bytes':sum(r['compressed_bytes'] for r in values),'saving_percent':100*(1-sum(r['compressed_bytes'] for r in values)/sum(r['raw_bytes'] for r in values)),'compression_ms_median':statistics.median(r['compression_ms_median'] for r in values),'decompression_ms_median':statistics.median(r['decompression_ms_median'] for r in values),'all_exact':all(r['exact_reconstruction'] for r in values)}))
