// Exercise the unchanged pinned artifact tool; normalize only random identities
// and cursor signatures at the observation boundary, never in the source store.
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
const {createArtifactToolDefinition}=await import(pathToFileURL(path.join(base,"dist/core/tools/artifact.js")));
const temporary=fs.mkdtempSync(path.join(os.tmpdir(),"ag1-artifact-tool-"));
const OriginalDate=globalThis.Date;
let result;
try {
 globalThis.Date=class extends OriginalDate {
  constructor(...args){super(...(args.length?args:[1000]));}
  static now(){return 1000;}
 };
 const store=new ArtifactStore(temporary,"session",{features:{typedEvidenceCursors:true,evidenceLeases:true,segmentedArtifactCatalog:true},now:()=>1000});
 const text="before\nneedle β 🦙\nafter\n";
 const record=store.putText(text,{kind:"tool-output",producer:{type:"system",name:"owned-artifact"},salientLines:["needle β 🦙"]});
 const canonicalId="art_00000000000000000000000000000001";
 const normalize=value=> {
  if(Array.isArray(value)) return value.map(normalize);
  if(value && typeof value==="object") return Object.fromEntries(Object.entries(value).map(([key,item])=>[key,normalize(item)]));
  if(typeof value!=="string") return value;
  if(/^ac[12]\./.test(value)) {const cursor=store.decodeReadCursor(value);return `cursor:${canonicalId}:${cursor.offset}:${cursor.maxBytes}:${cursor.kind}:${cursor.anchorLine??0}`;}
  return value.replaceAll(record.id,canonicalId).replace(/lease_[a-z0-9-]+/g,"lease_normalized");
 };
 const observed=output=> {
  const result=normalize(output);
  result.content=output.content.map(part=>{
   if(part.type!=="text") return normalize(part);
   const newline=part.text.indexOf("\n");
   try{return {...part,text:normalize(JSON.parse(part.text))};}catch{}
   if(newline>=0) {
    try{return {...part,text:{envelope:normalize(JSON.parse(part.text.slice(0,newline))),body:normalize(part.text.slice(newline+1))}};}catch{}
   }
   return normalize(part);
  });
  return result;
 };
 const actions=[
  {action:"list"},{action:"meta",id:record.id},{action:"read",id:record.id},
  {action:"read",id:record.id,maxBytes:8},{action:"read",id:record.id,offset:9,maxBytes:7},
  {action:"grep",id:record.id,query:"needle"},{action:"grep",id:record.id,query:"missing"},
  {action:"search",query:"needle"},{action:"search",query:"missing"},
  {action:"lease",id:record.id,reason:" Keep  decisive\n evidence "},
  {action:"leases"},{action:"release",id:record.id},{action:"leases"},
 ];
 const tool=createArtifactToolDefinition(store);
 const rows=[];
 for(const [index,args] of actions.entries()) {
  const output=await tool.execute(`fixture-${index}`,args);
  // Metrics affected by the optional SDK lexical skip index are not output
  // semantics. Native actual scan counts have their own storage contracts.
  if(args.action==="search") for(const key of ["scanned","candidates","bytesScanned"]) delete output.details[key];
  rows.push({id:`action-${index}`,args:normalize(args),expected:observed(output)});
 }
 result={version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files:["artifact-store","tools/artifact"].map(name=>({path:`packages/coding-agent/src/core/${name}.ts`,sha256:createHash("sha256").update(fs.readFileSync(path.join(base,`src/core/${name}.ts`))).digest("hex")})),text,record:normalize(record),rows,model_actions:0,network_actions:0};
}finally {globalThis.Date=OriginalDate;fs.rmSync(temporary,{recursive:true,force:true});}
const target=path.join(root,"fixtures/runtime/artifact-tool-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",artifact_tool_cases:result.rows.length,model_actions:0,network_actions:0}));
