import fs from 'node:fs';import path from 'node:path';import http from 'node:http';import zlib from 'node:zlib';import {createHash} from 'node:crypto';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');process.env.AG1_ALLOW_PRIVATE_WEBFETCH='1';
const {createWebFetchTool}=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/webfetch.js')));
const cwd=fs.mkdtempSync(path.join(root,'.local/p09-web-compression-'));const tool=createWebFetchTool({cwd});
const rows=[];const codecs={gzip:zlib.gzipSync,deflate:zlib.deflateSync,br:zlib.brotliCompressSync};
try {for(const [codec,encode]of Object.entries(codecs))for(const size of [180,1_000_001,...(codec==='gzip'?[1_000_000]:[])]){
 const input={codec,size,unit:'A supporting fact contains water, sunlight and the seasonal environment. 🦙\n'};
 const body=Buffer.from(input.unit.repeat(Math.ceil(size/Buffer.byteLength(input.unit)))).subarray(0,size);const compressed=encode(body);
 const server=http.createServer((_request,response)=>{response.writeHead(200,{'Content-Type':'text/plain','Content-Encoding':codec,'Content-Length':compressed.length});response.end(compressed);});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const origin=`http://127.0.0.1:${server.address().port}`;
 try {const output=await tool.execute(`compression-${codec}-${size}`,{url:origin+'/'},undefined,undefined,{cwd});rows.push({input:{...input,compressedBase64:compressed.toString('base64'),decodedSha256:createHash('sha256').update(body).digest('hex')},output:JSON.parse(JSON.stringify(output).replaceAll(origin,'http://fixture.invalid'))});}finally{await new Promise(resolve=>server.close(resolve));}
}}finally{fs.rmSync(cwd,{recursive:true,force:true});}
const result={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows};const target=path.join(root,'fixtures/tools/web-compression-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',compression_cases:rows.length,model_actions:0,external_network_actions:0}));
