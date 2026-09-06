// Real pinned ArtifactStore.grep supplies literal matches and surrounding ranges.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import {createHash} from "node:crypto";
import {fileURLToPath,pathToFileURL} from "node:url";
await import("../offline_guard.mjs");
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"../..");
const base=path.join(root,"reference/ag1-executable/packages/coding-agent");
const {ArtifactStore}=await import(pathToFileURL(path.join(base,"dist/core/artifact-store.js")));
const cases=[
 {id:"crlf",text:"before\r\nNeedle β 🦙\r\nafter\nlast",query:"needle"},
 {id:"sensitive",text:"Needle\nneedle\nNEEDLE\n",query:"needle",ignoreCase:false},
 {id:"consecutive",text:"needle one\nneedle two\nneedle three",query:"needle"},
 {id:"empty-lines",text:"\n\nneedle\n\n",query:"needle"},
 {id:"cr-at-eof",text:"before\nretained\r",query:"\r"},
 {id:"cr-before-lf",text:"stripped\r\n",query:"\r"},
 {id:"double-cr",text:"retained\r\r\n",query:"\r"},
 {id:"cap",text:"needle\nneedle\nneedle\nneedle\n",query:"needle",maxMatches:2},
 {id:"cross-line",text:"first\nsecond",query:"first\nsecond"},
 {id:"long-line-tail",prefix:"before\n",repeat:{text:"x",count:2100000},suffix:" needle\nafter\n",query:"needle"},
 {id:"split-utf8",repeat:{text:"x",count:65535},suffix:"🦙β NEEDLE\n",query:"β needle"},
 ...["ος","οσ","σ"].map(query=>({id:`sigma-${query}`,text:"ΟΣ\nΟΣΑ\nΣ\nΟ'Σ\nΟ'Σ'\nΟ'Σ'Α\nΟΣʰ\nΟΣʰΑ",query})),
 ...["ος","οσ"].map(query=>({id:`long-sigma-${query}`,prefix:"ΟΣ",repeat:{text:"\u0301",count:70000},suffix:"Α\nΟΣ\u0301\n",query})),
 {id:"expanded-lowercase",text:"İSTANBUL\nẞ\nI\n",query:"i\u0307s"},
 {id:"no-match",text:"Plain evidence",query:"missing"},
 {id:"empty",text:"",query:"missing"},
];
const rows=[];
for(const test of cases){
 const temporary=fs.mkdtempSync(path.join(os.tmpdir(),"ag1-artifact-scan-"));
 try{
  const text=test.text??((test.prefix??"")+(test.repeat?.text.repeat(test.repeat.count)??"")+(test.suffix??""));
  const store=new ArtifactStore(temporary,"fixture",{features:{typedEvidenceCursors:true},now:()=>1000});
  const record=store.putText(text,{kind:"text",producer:{type:"system",name:"owned-scan"}});
  const output=await store.grep(record.id,test.query,{ignoreCase:test.ignoreCase??true,maxMatches:test.maxMatches??100});
  const hits=output.matches.map(hit=>{const cursor=store.decodeReadCursor(hit.contextCursor);return {line:hit.line,text:hit.text,byteOffset:hit.byteOffset,context:{offset:cursor.offset,maxBytes:cursor.maxBytes,anchorLine:cursor.anchorLine}};});
  rows.push({...test,hits,truncated:output.truncated,bytes:Buffer.byteLength(text),sha256:createHash("sha256").update(text).digest("hex")});
 }finally{fs.rmSync(temporary,{recursive:true,force:true});}
}
const source_files=[{path:"packages/coding-agent/src/core/artifact-store.ts",sha256:createHash("sha256").update(fs.readFileSync(path.join(base,"src/core/artifact-store.ts"))).digest("hex")}];
const result={version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files,rows,model_actions:0,network_actions:0};
const target=path.join(root,"fixtures/runtime/artifact-scan-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",artifact_scan_cases:rows.length,model_actions:0,network_actions:0}));
