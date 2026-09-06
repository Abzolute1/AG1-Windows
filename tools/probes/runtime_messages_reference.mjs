// Real pinned SDK message conversion; no inference, filesystem effects or UI.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const relative = "packages/coding-agent/src/core/messages.ts";
const { convertToLlm } = await import(pathToFileURL(path.join(root,
  "reference/ag1-executable/packages/coding-agent/dist/core/messages.js")));
const inputs = [
  [{role:"compactionSummary",summary:"Remember exact evidence 🦀\nNext action",tokensBefore:5000,timestamp:100}],
  [{role:"branchSummary",summary:"Branch findings\n",fromId:"fork-point",timestamp:101}],
  [{role:"custom",content:"Injected context",display:false,details:{private:"retained only in journal"},timestamp:102}],
  [{role:"custom",content:[{type:"text",text:"image context"},{type:"image",data:"YQ==",mimeType:"image/png"}],timestamp:103}],
  [{role:"bashExecution",command:"check",output:"ok\n",exitCode:0,timestamp:104}],
  [{role:"bashExecution",command:"check",output:"",exitCode:2,timestamp:105}],
  [{role:"bashExecution",command:"check",output:"partial",exitCode:2,cancelled:true,truncated:true,fullOutputPath:"artifact.txt",timestamp:106}],
  [{role:"bashExecution",command:"private",output:"must stay out",excludeFromContext:true,timestamp:107},
   {role:"user",content:"Continue",timestamp:108}],
  [{role:"assistant",content:[{type:"toolCall",id:"call",name:"read",arguments:{path:"a"}}],timestamp:109},
   {role:"toolResult",toolCallId:"call",toolName:"read",content:[{type:"text",text:"evidence"}],timestamp:110}],
  [{role:"unknownExtensionRole",content:"retained but not sent",timestamp:111}],
];
const rows = inputs.map((messages, index) => {
  const original = structuredClone(messages);
  const output = convertToLlm(messages);
  assert.deepEqual(messages, original);
  return {id:`messages-${index}`,messages,output};
});
const result = {version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",
  source_files:[{path:relative,sha256:createHash("sha256").update(fs.readFileSync(path.join(root,"reference/ag1-executable",relative))).digest("hex")}],
  rows,model_actions:0,network_actions:0};
const target = path.join(root,"fixtures/runtime/messages-oracle.json");
if (process.argv.includes("--capture")) {
  assert(!fs.existsSync(target));
  fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");
} else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",message_cases:rows.length,model_actions:0,network_actions:0}));
