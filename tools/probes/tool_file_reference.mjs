import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..'),base=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools');
const write=await import(pathToFileURL(path.join(base,'write.js'))),edit=await import(pathToFileURL(path.join(base,'edit.js'))),append=await import(pathToFileURL(path.join(base,'append-tool.js'))),preview=await import(pathToFileURL(path.join(base,'edit-preview-tool.js')));
const inputs=JSON.parse(fs.readFileSync(path.join(root,'fixtures/tools/file-inputs.json'),'utf8'));
const privateRoot=fs.mkdtempSync(path.join(root,'.local/p09-files-'));
const rows=[];
try {
 for(const [i,input] of inputs.entries()){
  const cwd=path.join(privateRoot,String(i));fs.mkdirSync(cwd);
  const filename=path.join(cwd,input.args.path);
  if(input.original!==null){fs.mkdirSync(path.dirname(filename),{recursive:true});fs.writeFileSync(filename,input.original);fs.chmodSync(filename,0o750);}
  const defs={write:write.createWriteToolDefinition(cwd),edit:edit.createEditToolDefinition(cwd),append:append.createAppendToolDefinition(cwd),editApply:preview.createEditApplyToolDefinition(cwd),editPreview:preview.createEditPreviewToolDefinition(cwd)};
  let result;try{result={result:await defs[input.tool].execute(`fixture-${i}`,input.args)};}catch(error){result={error:error.message};}
  const output=JSON.parse(JSON.stringify(result).replaceAll(cwd,'/fixture'));
  const final=fs.existsSync(filename)?{bytes:fs.readFileSync(filename).toString('base64'),mode:fs.statSync(filename).mode&0o777}:null;
  rows.push({input,output,final});
 }
}finally{fs.rmSync(privateRoot,{recursive:true,force:true});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,rows};
const target=path.join(root,'fixtures/tools/file-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',file_cases:rows.length,model_actions:0,network_actions:0}));
