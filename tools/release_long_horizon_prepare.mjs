// P14 adapter preparation: original scenario functions remain byte-for-byte inputs.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import {pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';
const root=process.cwd(),out=path.resolve(process.argv[2] ?? '');
assert(process.argv[2], 'New private output directory required');
assert.equal(fs.realpathSync(path.dirname(out)), fs.realpathSync(path.join(root,'.local')), 'Use a new direct child of .local');
assert(!fs.existsSync(out), 'Existing evidence cannot be overwritten');
fs.mkdirSync(out,{mode:0o700});
const {default:ts}=await import(pathToFileURL(path.join(root,'product/apps/desktop/node_modules/typescript/lib/typescript.js')));
const sourcePath=path.join(root,'reference/ag1-executable/scripts/long-horizon-context-benchmark.ts');
const text=fs.readFileSync(sourcePath,'utf8');
const ast=ts.createSourceFile(sourcePath,text,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
const needed=new Set(['CHECKPOINT_SPECS','deterministicNoiseLine','buildRawArtifact','evidenceSchedule','historicalMessages','emptyUsage','containedTarget','runCommand','createWorkspace','llamaCppTokenizeUrl','countTokens','calibratedArtifactChars','artifactRoot','seedHistory','minimalResourceLoader','usageSince','changedFiles','executeCheckpoint','findMarkerArtifact','verifyArtifactSamples','directoryBytes']);
const fragments=[];
for(const node of ast.statements){
  const name=ts.isFunctionDeclaration(node)?node.name?.text:ts.isVariableStatement(node)?node.declarationList.declarations[0]?.name?.text:null;
  if(needed.has(name))fragments.push({name,text:node.getText(ast)});
}
assert.deepEqual(fragments.map(r=>r.name).sort(),[...needed].sort());
const hash=s=>crypto.createHash('sha256').update(s).digest('hex');
// Retain the original task expression too: driver changes must not simplify it.
let taskExpression;
function visit(node) {
  if(ts.isPropertyAssignment(node) && node.name.getText(ast)==='task' && node.initializer.getText(ast).includes('authoritative')) {
    assert.equal(taskExpression,undefined);taskExpression=node.initializer.getText(ast);
  }
  ts.forEachChild(node,visit);
}
visit(ast);assert(taskExpression);
const preamble=`import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve, sep } from 'node:path';
export function createBenchmark(bindings) {
const { ArtifactStore, SessionManager, SettingsManager, externalizeLargeToolResult, createExtensionRuntime, createAgentSession, fetch } = bindings;
`;
const generated=ts.transpileModule(preamble+fragments.map(r=>r.text).join('\n')+
  `\nfunction checkpointTask(index,spec,evidence) { return ${taskExpression}; }\nreturn { ${[...needed].join(', ')}, checkpointTask };\n}`,
  {compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText;
fs.writeFileSync(path.join(out,'fixture-generator.mjs'),generated,{flag:'wx',mode:0o600});
const {createBenchmark}=await import(pathToFileURL(path.join(out,'fixture-generator.mjs')));
// No fetch or session binding is supplied to these offline verifier controls.
const fixture=createBenchmark({});
fs.writeFileSync(path.join(out,'checkpoint-specs.json'),JSON.stringify(fixture.CHECKPOINT_SPECS,null,2),{flag:'wx',mode:0o600});
const workspace=path.join(out,'verifier-workspace');fs.mkdirSync(path.join(workspace,'src'),{recursive:true,mode:0o700});
const checks=[];
for(const spec of fixture.CHECKPOINT_SPECS){
 const file=path.join(workspace,spec.sourcePath),verifier=path.join(workspace,spec.verification);
 fs.writeFileSync(verifier,spec.verificationSource,{flag:'wx',mode:0o600});
 const positive=spec.id==='recovery'?'export function recoveryKey(service, attempt) { return String(service).toLowerCase() + "|" + attempt; }\n':spec.initialSource;
 fs.writeFileSync(file,positive,{mode:0o600});
 const good=spawnSync(process.execPath,[verifier],{cwd:workspace,encoding:'utf8'});assert.equal(good.status,0,good.stderr);
 fs.writeFileSync(file,spec.brokenSource);
 const bad=spawnSync(process.execPath,[verifier],{cwd:workspace,encoding:'utf8'});assert.notEqual(bad.status,0);
 fs.writeFileSync(file,spec.initialSource);
 checks.push({id:spec.id,verifier_sha256:hash(spec.verificationSource),initial_exit:good.status,broken_exit:bad.status});
 fs.writeFileSync(path.join(out,spec.id+'-good.log'),good.stdout+good.stderr,{flag:'wx'});
 fs.writeFileSync(path.join(out,spec.id+'-broken.log'),bad.stdout+bad.stderr,{flag:'wx'});
}
const schedule=fixture.evidenceSchedule(64,fixture.CHECKPOINT_SPECS);
assert.equal(schedule.size,12);
for(const [phase,item] of schedule){
 const raw=fixture.buildRawArtifact(phase,160000,item.line);
 assert(raw.includes(item.line));assert(raw.indexOf(item.line)>60000);assert(raw.length-raw.indexOf(item.line)>60000);
 assert.equal(raw,fixture.buildRawArtifact(phase,160000,item.line));
}
const report={status:'prepared-and-checked',release_acceptance:false,model_actions:0,scope:'Exact original fixture/verifier extraction with separate positive and negative controls. The recovery initial implementation is intentionally obsolete; its positive control follows the current contract, while extracted baseline bytes remain unchanged. No live session, token calibration or duration claim.',source:sourcePath,source_sha256:hash(text),task_expression_sha256:hash(taskExpression),generated_sha256:hash(generated),fragments:fragments.map(r=>({name:r.name,sha256:hash(r.text)})),checks};
fs.writeFileSync(path.join(out,'preparation.json'),JSON.stringify(report,null,2),{flag:'wx',mode:0o600});
console.log(JSON.stringify(report,null,2));
