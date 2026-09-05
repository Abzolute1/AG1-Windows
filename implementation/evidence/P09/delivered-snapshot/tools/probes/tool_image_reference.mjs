import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';import {createHash} from 'node:crypto';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..'),base=path.join(root,'reference/ag1-executable/packages/coding-agent/dist');
const read=await import(pathToFileURL(path.join(base,'core/tools/read.js'))),{loadPhoton}=await import(pathToFileURL(path.join(base,'utils/photon.js')));
const photon=await loadPhoton();assert(photon,'real reference Photon must load');
const dir=path.join(root,'fixtures/tools/images');const capture=process.argv.includes('--capture');
if(capture){
 const decoded=photon.PhotonImage.new_from_byteslice(fs.readFileSync(path.join(dir,'small.png')));
 const jpeg=Buffer.from(decoded.get_bytes_jpeg(80));decoded.free();
 const exif=Buffer.from('45786966000049492a0008000000010012010300010000000600000000000000','hex');
 const size=Buffer.alloc(2);size.writeUInt16BE(exif.length+2);
 const orientation=Buffer.concat([jpeg.subarray(0,2),Buffer.from([0xff,0xe1]),size,exif,jpeg.subarray(2)]);
 for(const [name,data] of [['small.jpg',jpeg],['orientation.jpg',orientation],['small.gif',Buffer.from('R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7','base64')],['invalid.png',Buffer.concat([Buffer.from('89504e470d0a1a0a','hex'),Buffer.alloc(16)])]]){
  const target=path.join(dir,name);assert(!fs.existsSync(target));fs.writeFileSync(target,data);
 }
}
const cwd=fs.mkdtempSync(path.join(root,'.local/p09-images-'));const rows=[];
try{
 for(const name of ['small.png','wide.png','tall.png','small.jpg','orientation.jpg','small.gif','invalid.png']){
  const bytes=fs.readFileSync(path.join(dir,name));fs.writeFileSync(path.join(cwd,name),bytes);
  for(const resize of [true,false]){
   const definition=read.createReadToolDefinition(cwd,{autoResizeImages:resize});
   let output;try{output={result:await definition.execute('image',{path:name})};}catch(error){output={error:error.message};}
   rows.push({name,resize,input_sha256:createHash('sha256').update(bytes).digest('hex'),output:JSON.parse(JSON.stringify(output))});
  }
 }
}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,rows};const target=path.join(root,'fixtures/tools/image-oracle.json');
if(capture){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',image_cases:rows.length,model_actions:0,network_actions:0}));
