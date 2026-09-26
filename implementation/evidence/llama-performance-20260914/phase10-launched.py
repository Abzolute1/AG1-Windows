import json
from pathlib import Path
from bench import run
p=Path(__file__).parent
for name in ['affinity-t6-unbound-a', 'affinity-t6-ccd1', 'affinity-t6-ccd0', 'affinity-t4-ccd1', 'affinity-t12-physical', 'affinity-t6-unbound-b', 'affinity-cow-ccd1', 'affinity-anon-ccd1', 'affinity-host-ccd1']:
 run(json.loads((p/(name+".json")).read_text()))
