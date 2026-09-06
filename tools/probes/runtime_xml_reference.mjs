// Execute the pinned SDK XML parser and recovery guidance unchanged.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const base=path.join(root,'reference/ag1-executable/packages/coding-agent');
const sdk=await import(pathToFileURL(path.join(base,'dist/core/xml-tool-parser.js')));
const tools=['read','write','bash','custom.name'];
const cases=[
 ['plain','No executable syntax'],
 ['bare','<read><path>a.ts</path><offset>2</offset></read>'],
 ['param','<write><param name="path">a.ts</param><param name="content"> x </param></write>'],
 ['hyphen','<tool_call><function-read><parameter-path> a.ts </parameter-path></function-read></tool_call>'],
 ['equals','<tool_call><function=write><parameter=path>a.ts</parameter><parameter=content>hello</parameter></function></tool_call>'],
 ['wrapped-param','<tool_call><function-bash><param name="command">pwd</param></function-bash></tool_call>'],
 ['equals-param','<tool_call><function=bash><param name="command">pwd</param></function></tool_call>'],
 ['empty-args','<read>no arguments</read>'],
 ['unknown','<delete><path>a.ts</path></delete>'],
 ['fenced','```xml\n<write><path>a.ts</path><content>example</content></write>\n```'],
 ['mixed-fenced','```\n<read><path>hidden</path></read>\n```\n<read><path>visible</path></read>'],
 ['unclosed-fence','```\n<read><path>visible</path></read>'],
 ['wrapped-overlap','<tool_call><function-write><parameter-path>a.ts</parameter-path><parameter-content><read><path>example</path></read></parameter-content></function-write></tool_call>'],
 ['multiple','<read><path>first</path></read><bash><command>pwd</command></bash>'],
 ['format-order','<read><path>first</path></read><tool_call><function-bash><parameter-command>pwd</parameter-command></function-bash></tool_call>'],
 ['case','<READ><PATH> a.ts </PATH></READ>'],
 ['attributes','<read data-x="1"><path flag="yes">a.ts</path></read>'],
 ['special-name','<custom.name><value>one</value></custom.name>'],
 ['duplicate-param','<read><path>old</path><path>new</path></read>'],
 ['malformed','<read><path>a.ts</path>'],
 ['mismatched-param','<read><path>a.ts</other></read>'],
 ['no-active','<read><path>a.ts</path></read>',[]],
 ['nested-argument','<write><path>a.ts</path><content><span>hello</span></content></write>'],
 ['param-precedence','<read><param name="path">one</param><offset>2</offset></read>'],
 ['bare-function-ignored','<function-read><parameter-path>a.ts</parameter-path></function-read>'],
 ['mixed-wrapper','<tool_call><function=read><parameter=path>a.ts</parameter></function><function-bash><parameter-command>pwd</parameter-command></function-bash></tool_call>'],
];
const rows=cases.map(([id,text,knownTools=tools])=>({id,text,knownTools,calls:sdk.parseXmlToolCalls(text,knownTools)}));
const analyses=[];
for(const [callCount,assistantTextLength,assistantStopReason,iteration,names] of [
 [1,20,'stop',0,['read']],[1,20,'stop',0,['write']],[1,20,'stop',0,['append']],
 [2,20,'stop',0,['read','bash']],[1,4000,'stop',0,['read']],[1,12000,'stop',0,['read']],
 [1,20,'length',0,['read']],[1,20,'max_tokens',0,['read']],[1,20,'stop',1,['read']],
 [2,4000,'length',2,['write']],[1,3999,'stop',0,['read']],[1,11999,'stop',0,['bash']],
 [1,20,undefined,0,['read']],
]) {
 const input={callCount,assistantTextLength,assistantStopReason,iteration};
 const analysis=sdk.analyzeXmlFallbackTurn(input);
 const guidance=sdk.buildXmlFallbackRecoveryPrompt(analysis,{toolNames:names});
 analyses.push({input,names,analysis,...(guidance===undefined?{}:{guidance})});
}
const source='packages/coding-agent/src/core/xml-tool-parser.ts';
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',source_files:[{path:source,sha256:createHash('sha256').update(fs.readFileSync(path.join(base,'src/core/xml-tool-parser.ts'))).digest('hex')}],rows,analyses,model_actions:0,network_actions:0}));
const target=path.join(root,'fixtures/runtime/xml-tool-oracle.json');
if(process.argv.includes('--capture')) { assert(!fs.existsSync(target)); fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n'); }
else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',parse_cases:rows.length,recovery_cases:analyses.length,model_actions:0,network_actions:0}));
