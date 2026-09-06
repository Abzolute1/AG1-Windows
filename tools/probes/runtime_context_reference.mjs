// Execute the pinned AgentSession request-state closure and installed transform.
// Compiler/projection behavior has separate P08 oracles; no model is invoked here.
import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
await import("../offline_guard.mjs");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const base = path.join(root, "reference/ag1-executable/packages/coding-agent");
const names = ["agent-session", "task-state", "prompt-sections", "episode-retrieval", "runtime-observations", "inspection-evidence", "compaction/index"];
const modules = await Promise.all(names.map(name => import(pathToFileURL(path.join(base, `dist/core/${name}.js`)))));
const bindings = Object.assign({}, ...modules);
const compiled = fs.readFileSync(path.join(base, "dist/core/agent-session.js"), "utf8");
const start = compiled.indexOf("        const buildRequestContext = () => {");
const end = compiled.indexOf("        let requestId = 0;", start);
assert(start > 0 && end > start);
// Evaluate the unchanged closure, with its actual imported source functions.
const build = new Function(...Object.keys(bindings), compiled.slice(start, end) + "return buildRequestContext.call(this);");
const fixture = JSON.parse(fs.readFileSync(path.join(root, "fixtures/context/projection-inputs.json"), "utf8"));
const user = { role: "user", content: "Continue the desktop task", timestamp: 0 };
const rounds = [{role:"assistant",content:[{type:"toolCall",id:"call-one",name:"read",arguments:{path:"a.ts"}}],timestamp:1},
  {role:"toolResult",toolCallId:"call-one",toolName:"read",content:[{type:"text",text:"evidence"}],timestamp:2}];
const source = `file:a.ts@sha256:${"a".repeat(64)}`;
const cases = [
  {id:"empty",branch:[],messages:[user],verdicts:{}},
  ...fixture.tasks.map((entry,index)=>({id:`task-${index}`,branch:[{type:"custom",customType:"task_state",data:entry.state}],messages:[user,...rounds],
    verdicts:Object.fromEntries(Object.entries(entry.verdicts).map(([source,v])=>[source,{status:v.status,...(v.currentRevision?{currentSha256:v.currentRevision}:{})}]))})),
  ...["current", "stale"].map(status=>({id:`file-${status}`,branch:[{type:"custom",customType:"task_state",data:{...fixture.tasks[0].state,entries:[{id:"ts_00000001",kind:"FACT",statement:"File evidence",status:"active",sources:[source],createdAt:fixture.tasks[0].state.updatedAt,updatedAt:fixture.tasks[0].state.updatedAt}]}}],messages:[user],verdicts:{[source]:{status,currentSha256:(status==="current"?"a":"b").repeat(64)}}})),
  {id:"blocks",branch:[{type:"custom",customType:"task_state",data:fixture.tasks[0].state}],messages:[{...user,content:[{type:"text",text:"Continue"},{type:"image",data:"YQ==",mimeType:"image/png"}]},...rounds],verdicts:{}},
  {id:"no-user",branch:[{type:"custom",customType:"task_state",data:fixture.tasks[0].state}],messages:rounds,verdicts:{}},
  {id:"runtime-inspection",branch:[...fixture.runtimes[0],...fixture.inspections[0].evidence.map(data=>({type:"custom",customType:"inspection_evidence",data}))],messages:[user],verdicts:fixture.inspections[0].verdicts},
];
const rows=[];
for(const entry of cases) {
  const messages=structuredClone(entry.messages);
  const host={contextWindow:0,_contextCompilerEnabled:false,agent:{state:{messages,systemPrompt:"fixture",tools:[]}},
    sessionManager:{getBranch:()=>entry.branch},artifactStore:{},_sourceRevisionValidator:{validate:source=>entry.verdicts[source]},
    _extractUserMessageText:bindings.AgentSession.prototype._extractUserMessageText,_rehydrateGodotVisualArtifacts:values=>values};
  const prepared=build.call(host,...Object.values(bindings));
  bindings.AgentSession.prototype._installCacheFriendlyWorkingStateTransform.call(host);
  const output=await host.agent.transformContext(prepared.context.messages);
  assert.deepEqual(messages,entry.messages);
  rows.push({...entry,tail:host._requestWorkingStateTail?.content??null,output});
}
const source_files=names.map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash("sha256").update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest("hex")}));
const result={version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files,rows,model_actions:0,network_actions:0};
const target=path.join(root,"fixtures/runtime/context-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",context_cases:rows.length,model_actions:0,network_actions:0}));
