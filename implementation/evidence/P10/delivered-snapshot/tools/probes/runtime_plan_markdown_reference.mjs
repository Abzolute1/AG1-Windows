// Replay literal planning inputs extracted from the pinned tests through the
// real exported functions. This is an oracle capture, not a Vitest assertion run.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable');
const ts=(await import(pathToFileURL(path.join(base,'node_modules/typescript/lib/typescript.js')))).default;
const ref=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/modes/interactive/interactive-plan-artifacts.js')));
const {InteractiveMode}=await import(pathToFileURL(path.join(base,'packages/coding-agent/dist/modes/interactive/interactive-mode.js')));
const sources=['packages/coding-agent/src/modes/interactive/interactive-plan-artifacts.ts','packages/coding-agent/src/modes/interactive/interactive-mode.ts','packages/coding-agent/test/interactive-plan-artifacts.test.ts'];
const source=fs.readFileSync(path.join(base,sources[2]),'utf8');
const parsed=ts.createSourceFile('planning.test.ts',source,ts.ScriptTarget.Latest,true);
const now='2026-09-06T00:00:00.000Z';
const fixed=plan=>plan?{...plan,planId:'plan-a'}:null;
const rows=[];
function visit(node){
  if(ts.isCallExpression(node)&&node.expression.getText(parsed)==='extractInteractivePlanArtifact'&&ts.isObjectLiteralExpression(node.arguments[0])){
    const properties=node.arguments[0].properties;
    const literal=name=>{const p=properties.find(p=>ts.isPropertyAssignment(p)&&p.name.getText(parsed)===name);return p&&(ts.isStringLiteral(p.initializer)||ts.isNoSubstitutionTemplateLiteral(p.initializer))?p.initializer.text:undefined;};
    const goal=literal('goal'),assistantText=literal('assistantText');
    if(goal!==undefined&&assistantText!==undefined&&!properties.some(p=>p.name?.getText(parsed)==='previous')){
      const input={goal,assistantText};
      rows.push({line:parsed.getLineAndCharacterOfPosition(node.getStart(parsed)).line+1,input,result:fixed(ref.extractInteractivePlanArtifact({...input,now}))});
    }
  }
  ts.forEachChild(node,visit);
}
visit(parsed);
assert(rows.length>=15);
const goals=[...new Set(rows.map(r=>r.input.goal).concat(['','Build an API','plan a snake game in TypeScript for the browser','plan the current runtime bootstrap refactor',"Hey, let's enter planmode for a snake game!",'Start planning mode for a browser snake game in TypeScript','Inspect src/api.ts','Just an idea']))];
const fallback=goals.map(goal=>({goal,interview:ref.shouldStartWithFallbackPlanningInterview(goal),inspect:InteractiveMode.prototype.shouldAllowPlanningInspection.call({},goal),inspectPrevious:InteractiveMode.prototype.shouldAllowPlanningInspection.call({},goal,{previousArtifact:{}}),result:fixed(ref.buildFallbackInteractivePlanArtifact({goal,rawPlan:'Sure, let us plan.',now}))}));
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:sources.map(file=>({path:file,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,file))).digest('hex')})),rows,fallback,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/plan-markdown-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',markdown:rows.length,fallback:fallback.length,model_actions:0,network_actions:0}));
