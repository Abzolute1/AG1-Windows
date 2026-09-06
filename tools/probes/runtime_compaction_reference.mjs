// Execute pinned coordinator checkpoint/provenance helpers without inference.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const base = path.join(root,"reference/ag1-executable/packages/coding-agent");
const { AgentSessionCompactionCoordinator } = await import(pathToFileURL(path.join(base,"dist/core/agent-session-compaction-coordinator.js")));
const { attachCompactionProvenance } = await import(pathToFileURL(path.join(base,"dist/core/compaction-artifact.js")));
const artifact = {id:"art_"+"1".repeat(32),sha256:"2".repeat(64),kind:"compaction-trajectory",bytes:100};
const rows = [];
for (const populated of [false,true]) {
  const preparation = {firstKeptEntryId:"kept",tokensBefore:50000,workingMemory:{version:1,fixture:"opaque retained state"},
    fileOps:{read:populated?["z","a","z"]:[],written:["written"],edited:populated?["b","a"]:[]}};
  const cause = populated ? "oversized input: "+"long diagnostic ".repeat(100) : "summary timed out";
  const input = {...preparation,fileOps:Object.fromEntries(Object.entries(preparation.fileOps).map(([key,values])=>[key,new Set(values)]))};
  const output = AgentSessionCompactionCoordinator.prototype.createPersistentEmergencyFallback.call(null,input,artifact,cause);
  rows.push({kind:"fallback",preparation,cause,artifact,output});
}
for (const details of [undefined,{readFiles:["a"]},["opaque"],null]) {
  const result={summary:"Model summary",firstKeptEntryId:"kept",tokensBefore:50000,...(details===undefined?{}:{details})};
  rows.push({kind:"provenance",result,artifact,output:attachCompactionProvenance(result,artifact)});
}
const source_files=["agent-session-compaction-coordinator.ts","compaction-artifact.ts"].map(name=>({path:"packages/coding-agent/src/core/"+name,
  sha256:createHash("sha256").update(fs.readFileSync(path.join(base,"src/core",name))).digest("hex")}));
const result={version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files,rows,model_actions:0,network_actions:0};
const target=path.join(root,"fixtures/runtime/compaction-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",compaction_cases:rows.length,model_actions:0,network_actions:0}));
