// Execute unchanged ArtifactStore.search, normalizing only fixture identity/time.
import fs from "node:fs";import os from "node:os";import path from "node:path";import assert from "node:assert/strict";import {createHash} from "node:crypto";import {fileURLToPath,pathToFileURL} from "node:url";
await import("../offline_guard.mjs");
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),"../..");
const base=path.join(root,"reference/ag1-executable/packages/coding-agent");
const {ArtifactStore}=await import(pathToFileURL(path.join(base,"dist/core/artifact-store.js")));
const inputs=[
 {text:"needle one\nneedle two\n",kind:"text"},
 {text:"needle special\n",kind:"tool-output",salientLines:["needle special"]},
 {text:"needle one\nneedle two\n",kind:"text"},
 {text:"NEEDLE CAPITAL\n",kind:"text"},
 {text:"Unrelated retained evidence\n",kind:"text"},
];
const cases=[{id:"ranked",query:"needle"},{id:"bounded",query:"needle",maxArtifacts:1,maxMatchesPerArtifact:1},{id:"kind",query:"needle",kind:"text"},{id:"sensitive",query:"needle",ignoreCase:false},{id:"no-match",query:"absent"}];
const temporary=fs.mkdtempSync(path.join(os.tmpdir(),"ag1-artifact-search-"));let result;
try{
 let serial=0;
 const OriginalDate=globalThis.Date;
 globalThis.Date=class extends OriginalDate {
  constructor(...args){super(...(args.length?args:[1000+serial*1000]));}
  static now(){return 1000+serial*1000;}
 };
 try {
 const store=new ArtifactStore(temporary,"parent",{features:{typedEvidenceCursors:true,segmentedArtifactCatalog:true},now:()=>1000+serial*1000});
 // Leave the store and its random identities untouched. Normalize identity
 // only at the observation boundary, after actual catalog lookup and ranking.
 const records=inputs.map(input=>{serial++;return store.putText(input.text,{kind:input.kind,producer:{type:"system",name:"owned-search"},...(input.salientLines?{salientLines:input.salientLines}:{})});});
 const ids=new Map(records.map((record,index)=>[record.id,`art_${(index+1).toString(16).padStart(32,"0")}`]));
 const rows=[];
 for(const input of cases){
  const found=await store.search(input.query,input);
  rows.push({...input,matches:found.matches.map(match=>({id:ids.get(match.record.id),occurrenceCount:match.occurrenceCount,totalMatches:match.totalMatches,hits:match.matches.map(hit=>({line:hit.line,text:hit.text,byteOffset:hit.byteOffset}))})),scannedArtifacts:found.scannedArtifacts,truncated:found.truncated});
 }
 result={version:1,source_commit:"e30f0b4b238ee5618928bcd17b4571eb046e7038",source_files:[{path:"packages/coding-agent/src/core/artifact-store.ts",sha256:createHash("sha256").update(fs.readFileSync(path.join(base,"src/core/artifact-store.ts"))).digest("hex")}],inputs:inputs.map((input,index)=>({text:input.text,record:{...records[index],id:ids.get(records[index].id)}})),rows,model_actions:0,network_actions:0};
 } finally {globalThis.Date=OriginalDate;}
}finally{fs.rmSync(temporary,{recursive:true,force:true});}
const target=path.join(root,"fixtures/runtime/artifact-search-oracle.json");
if(process.argv.includes("--capture")){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+"\n");}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,"utf8")));
console.log(JSON.stringify({status:"passed",artifact_search_cases:result.rows.length,model_actions:0,network_actions:0}));
