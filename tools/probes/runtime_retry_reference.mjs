import fs from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import {createHash} from "node:crypto";
import {fileURLToPath,pathToFileURL} from "node:url";
await import("../offline_guard.mjs");
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"../..");
const base=path.join(root,"reference/ag1-executable/packages");
const {AgentSessionRetryCoordinator}=await import(pathToFileURL(path.join(base,"coding-agent/dist/core/agent-session-retry-coordinator.js")));
const {classifyRequestFailure,recommendProviderBackoffMs}=await import(pathToFileURL(path.join(base,"coding-agent/dist/core/request-failure.js")));
const {isContextOverflow}=await import(pathToFileURL(path.join(base,"ai/dist/utils/overflow.js")));
const coordinator=Object.create(AgentSessionRetryCoordinator.prototype);
coordinator.options={getContextWindow:()=>10000};
const errors=["server overloaded","Provider stop_reason: network_error","Provider finish_reason: content_filter timeout",
 "Provider response status: failed 503","OpenAI API error (401) overloaded","OpenAI API error (429)","API error (500)","API error (404)",
 "invalid api key","OAuth credential expired","retry with api key","ResourceExhausted: Worker local total request limit reached (32/32)",
 "Worker local total request limit reached (80/32)","quota 1/0","quota 1/3","429 too many requests","connection refused", "fetch failed",
 "prompt is too long: 15000 tokens > 10000 maximum","request_too_large","input is too long for requested model",
 "Your input exceeds the context window","input token count exceeds the maximum","maximum prompt length is 10000",
 "reduce the length of the messages","maximum context length is 10000 tokens","exceeds the limit of 10000",
 "exceeds the available context size","greater than the context length","context window exceeds limit","exceeded model token limit",
 "too large for model with 10000 maximum context length","model_context_window_exceeded","prompt too long; exceeded max context length",
 "context_length_exceeded","too many tokens","token limit exceeded","400 status code (no body)","413 (no body)",
 "Throttling error: Too many tokens","Service unavailable: too many tokens","rate limit: too many tokens per minute",
 "Provider finish_reason: network_error context_length_exceeded","unknown failure"];
const rows=[];
for(const provider of ["owned-fixture","llamacpp"])for(const errorMessage of errors){
 const message={provider,model:"fixture",stopReason:"error",errorMessage,usage:{input:0,cacheRead:0}};
 rows.push({message,window:10000,kind:classifyRequestFailure(message)?.kind??null,overflow:isContextOverflow(message,10000),retryable:coordinator.isRetryableError(message),
  backoff:[1,2,3].map(attempt=>({attempt,base:2000,delay:recommendProviderBackoffMs(errorMessage,attempt,2000)}))});
}
for(const stopReason of ["stop","length","aborted"])for(const cacheRead of [0,2]){
 const message={provider:"owned-fixture",model:"fixture",stopReason,usage:{input:9999,cacheRead}};
 rows.push({message,window:10000,kind:null,overflow:isContextOverflow(message,10000),retryable:coordinator.isRetryableError(message),backoff:[]});
}
const source_files=["coding-agent/src/core/agent-session-retry-coordinator.ts","coding-agent/src/core/request-failure.ts","coding-agent/src/core/local-inference.ts","ai/src/utils/overflow.ts"].map(name=>({path:"packages/"+name,sha256:createHash("sha256").update(fs.readFileSync(path.join(base,name))).digest("hex")}));
const result={version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files,rows,model_actions:0,network_actions:0};
const target=path.join(root,"fixtures/runtime/retry-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",retry_cases:rows.length,model_actions:0,network_actions:0}));
