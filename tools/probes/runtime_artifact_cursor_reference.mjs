// Capture exact signed cursor bytes from the unchanged pinned ArtifactStore.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const base = path.join(root, "reference/ag1-executable/packages/coding-agent");
const {ArtifactStore} = await import(pathToFileURL(path.join(base, "dist/core/artifact-store.js")));
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "ag1-artifact-cursors-"));
const id = "art_00000000000000000000000000000001";
const ranges = [[0,29,2],[4,32772,3],[512,33281,4],[512,512,5],[0,40000,6],[7,8,4294967295]];
let rows;
try {
 const store = new ArtifactStore(temporary,"parent",{features:{typedEvidenceCursors:true}});
 fs.writeFileSync(path.join(temporary,"cursor-key-v1"),Buffer.alloc(32,7),{mode:0o600});
 rows = ranges.map(([start,end,anchor])=>({id,start,end,anchor,cursor:store.createContextCursor(id,start,end,anchor)}));
} finally { fs.rmSync(temporary,{recursive:true,force:true}); }
const source_files = [{path:"packages/coding-agent/src/core/artifact-store.ts",sha256:createHash("sha256").update(fs.readFileSync(path.join(base,"src/core/artifact-store.ts"))).digest("hex")}];
const result = {version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files,rows,model_actions:0,network_actions:0};
const target = path.join(root,"fixtures/runtime/artifact-cursor-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",artifact_cursor_cases:rows.length,model_actions:0,network_actions:0}));
