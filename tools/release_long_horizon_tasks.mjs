// Serialize the unchanged reference task expression for the Rust driver.
import fs from 'node:fs';import crypto from 'node:crypto';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const [generated,expected,seedPath,output]=process.argv.slice(2);
assert.equal(crypto.createHash('sha256').update(fs.readFileSync(generated)).digest('hex'),expected);
const {createBenchmark}=await import(pathToFileURL(generated));const b=createBenchmark({});
const seed=JSON.parse(fs.readFileSync(seedPath));
const specs=b.CHECKPOINT_SPECS.map((s,index)=>({...s,task:b.checkpointTask(index,s,b.findMarkerArtifact(seed.artifacts,s.marker))}));
fs.writeFileSync(output,JSON.stringify({system_prompt:b.minimalResourceLoader().getSystemPrompt(),specs},null,2),{flag:'wx',mode:0o600});
