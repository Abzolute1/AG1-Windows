// Execute the unchanged pinned memory tool in disposable directories, including
// its real reducers, file locks, presentation and source revision validator.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const base = path.join(root, "reference/ag1-executable/packages/coding-agent");
const load = name => import(pathToFileURL(path.join(base, `dist/core/${name}.js`)));
const [{createMemoryToolDefinition}, {saveMemoryStore, loadMemoryStore}, {recordMemorySelection, clearMemorySelections}] = await Promise.all([load("tools/memory"),load("memory-store"),load("memory-context")]);
const now = 1788652800000;
const entry = (id, text, other = {}) => ({id,kind:"fact",text,createdAt:now-86400000,priority:"normal",...other});
const one = {version:3,entries:[entry("m1","Native runtime architecture",{kind:"decision",priority:"core"})]};
const two = {version:3,entries:[...one.entries,entry("m2","Native runtime tests")]};
const hot = {version:3,entries:Array.from({length:40},(_,i)=>entry(`m${i+1}`,`Retained native note ${i+1}`,{createdAt:now-i*1000}))};
const digest = createHash("sha256").update("owned source\n").digest("hex");
const source = `file:source.ts@sha256:${digest}`;
const provenance = {version:3,entries:[entry("m1","Component behavior",{component:"Native",sources:[source]})]};
const cases = [
 {id:"empty-list",store:{version:3,entries:[]},args:{action:"list"}},
 {id:"add",store:one,args:{action:"add",kind:"style",text:"  Keep native\n boundaries explicit  ",priority:"core"}},
 {id:"duplicate",store:one,args:{action:"add",kind:"fact",text:"Native runtime architecture"}},
 {id:"update",store:two,args:{action:"update",id:"m2",text:"Updated native runtime tests",priority:"core",component:"Runtime"}},
 {id:"remove",store:two,args:{action:"remove",id:"m2"},loaded:["m1","m2"]},
 {id:"clear",store:two,args:{action:"clear"}},
 {id:"search",store:two,args:{action:"search",query:" runtime tests "}},
 {id:"no-match",store:two,args:{action:"search",query:"absentphrase"}},
 {id:"archive",store:hot,args:{action:"add",kind:"fact",text:"New native note"}},
 {id:"archived-search",store:{version:3,entries:[one.entries[0]],archived:[entry("m2","Cold runtime tests")]},args:{action:"search",query:"runtime tests"}},
 {id:"loaded-list",store:{...two,archived:[entry("m3","Archived note")]},loaded:["m1","m3"],args:{action:"list"}},
 ...["current","stale","missing"].map(status=>({id:`provenance-${status}`,store:provenance,args:{action:"list"},status,source})),
 {id:"near-capacity",store:hot,args:{action:"list"}},
 {id:"invalid-search",store:one,args:{action:"search",query:" "}},
 {id:"invalid-remove",store:one,args:{action:"remove"}},
 {id:"invalid-update",store:one,args:{action:"update",id:"m1"}},
 {id:"invalid-provenance",store:one,args:{action:"add",kind:"fact",text:"Unattributed component claim",component:"Runtime"}},
];
const rows = [];
const originalNow = Date.now;
try {
 Date.now = () => now;
 for (const test of cases) {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), "ag1-memory-oracle-"));
  try {
   if(test.status && test.status!=="missing") fs.writeFileSync(path.join(cwd,"source.ts"),test.status==="current"?"owned source\n":"changed source\n");
   saveMemoryStore(cwd,structuredClone(test.store));
   clearMemorySelections();
   const loaded = test.loaded ?? [];
   recordMemorySelection(cwd,{entries:[...test.store.entries,...(test.store.archived??[])].filter(e=>loaded.includes(e.id)),storedCount:test.store.entries.length});
   let result, error;
   try { result = await createMemoryToolDefinition(cwd).execute("owned-memory",structuredClone(test.args)); }
   catch(e) { error=String(e.message); }
   rows.push({...test,now,loaded,...(error?{error}:{result}),after:loadMemoryStore(cwd)});
  } finally { fs.rmSync(cwd,{recursive:true,force:true}); }
 }
} finally { Date.now=originalNow; clearMemorySelections(); }
const source_files = ["tools/memory", "memory-store", "memory-context", "source-revision"].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash("sha256").update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest("hex")}));
const result = {version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files,rows,model_actions:0,network_actions:0};
const target = path.join(root,"fixtures/runtime/memory-oracle.json");
if(process.argv.includes("--capture")){ assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n"); }
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",memory_tool_cases:rows.length,model_actions:0,network_actions:0}));
