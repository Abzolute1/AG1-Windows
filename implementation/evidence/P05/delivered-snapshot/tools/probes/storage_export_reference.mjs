// Verify a Rust compatible export with the unchanged SDK, in a disposable export destination.
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {pathToFileURL} from 'node:url';
const root=path.resolve(import.meta.dirname,'../..'),reference=root+'/reference/ag1-executable';
const {SessionManager}=await import(pathToFileURL(reference+'/packages/coding-agent/dist/core/session-manager.js'));
const {ArtifactStore}=await import(pathToFileURL(reference+'/packages/coding-agent/dist/core/artifact-store.js'));
const exported=path.resolve(process.argv[2]),expected=JSON.parse(fs.readFileSync(process.argv[3]));
assert(exported.startsWith(root+'/.local/'));
const manager=SessionManager.open(exported+'/compatible/sessions/s.jsonl',exported+'/compatible/sessions');
assert.equal(manager.getEntries().length,expected.entries);assert.deepEqual(manager.getEntries()[0].data,{unknown:'preserved'});
const artifacts=new ArtifactStore(exported+'/compatible/artifacts/s','s',{features:{typedEvidenceCursors:true,evidenceLeases:true},now:()=>0});
assert(artifacts.read(expected.artifact).text.startsWith('artifact 7\nneedle evidence\n'));
assert.equal(artifacts.createReadCursor(expected.artifact,1,5),expected.cursor);
assert.equal(artifacts.readCursor(expected.cursor).text,'rtifa');
assert.equal(artifacts.listEvidenceLeases().length,expected.leases);
console.log(JSON.stringify({status:'passed',original_sdk_resume:true,artifact_bytes:true,cursor_compatible:true,lease_compatible:true,model_actions:0}));
