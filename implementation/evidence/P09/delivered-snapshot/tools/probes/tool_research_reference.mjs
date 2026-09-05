import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const output=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/research-output.js')));
const intent=await import(pathToFileURL(path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools/research-intent.js')));
const rows=[];function probe(name,args){const fn=output[name]??intent[name];rows.push({name,args,result:fn(...args)??null});}
for(const q of ['plain gardening','latest React API hook parameters','OpenAI incident 2026','repo/file.js guide','useEffect','useEffect what it does','pricing model id','deep dive llama','summarize documentation','packages source','security advisory API','React released','what does this hook return','current price','node traceback']) {probe('detectResearchQueryIntent',[q]);probe('isCodeLookupQuery',[q]);probe('shouldUseRichResearchFetch',[q,intent.detectResearchQueryIntent(q)]);}
const usages=[undefined,{}, {contextWindow:32000},{contextWindow:32001},{contextWindow:80000},{contextWindow:80001},{percent:35},{percent:79},{percent:80},{contextWindow:90000,percent:20}];
for(const usage of usages){const args=usage?[usage]:[];probe('deriveResearchCompressionProfile',args);for(const richFetch of [false,true])probe('getEffectiveFetchCompressionProfile',[output.deriveResearchCompressionProfile(usage),{intent:'general',richFetch}]);}
for(const max of [0,1,2,7,12,40])for(const text of [' short \n text ','🦙🦙🦙🦙🦙','\ufeff alpha\t beta\u00a0 gamma']){probe('truncateResearchSnippet',[text,max]);probe('limitPreviewLines',[text+'\nsecond long line\n',2,max]);}
const documents=[
 '<html><title> fallback title </title><meta property="og:title" content=" Preferred   title "><meta name="description" content=" metadata description "><body>body<main> main area <article> article content </article></main></body></html>',
 '<main> \n </main><body> fallback body </body>', '<meta content="reverse order ignored" name="description"><title>nested <b>ignored</b></title>', '<title>  A &amp; B  </title>', '<article>first</article><article>second</article>', 'plain text',
];for(const html of documents)for(const name of ['extractResearchPrimaryHtml','extractResearchDocumentTitle','extractResearchMetaDescription'])probe(name,[html]);
const texts=[
 'Just a moment\nChecking your browser. Enable JavaScript to continue.',
 'Title\nA short sentence.',
 '# useEffect — React\nAPI Reference\npageCopy\nuseEffect(setup, dependencies) lets you synchronize a component with an external system.\nThe setup parameter returns an optional cleanup function.\nFor example, useEffect(() => connect(), []) runs once after mounting.\nAdded in React 16.8.\nCopy page\n- Reference',
 'import widget from "library";\nconst thing = makeThing();\nreturn thing;\nexport function main() {\nwidget.run();',
 'OpenAI confirmed an incident in August 2026.\nResearchers found source code exposed through an npm package.\nAffected users were advised to upgrade after the vendor removed the package.',
 'Pricing for model llama-fixture: input $2 per million tokens; output $4 per million tokens. Updated August 2026.',
 'Version 2.1 released September 2026. Added a feature and fixed a bug. Migration notes explain the breaking change.',
 'TypeError: missing argument occurs when the option is undefined. The workaround is to set the argument in version 2.1.',
 'Subscribe\nCookie policy\n# Gardening\nPlants grow when sunlight and water meet their seasonal needs.\nPlants grow when sunlight and water meet their seasonal needs.\nUse compost because it improves soil structure and water retention.\n- A second useful observation contains more than twenty four characters.',
 '🦙'.repeat(140)+'\n'+('An informative line contains supporting explanation. '.repeat(16)),
];
for(const markdown of texts){for(const focus of [[],['parameter','once']]){probe('buildResearchPreview',[markdown,4,120,focus]);probe('extractResearchFacts',[markdown,4,120,focus]);}for(const kind of ['current_events','docs','pricing','release','error'])probe('assessResearchCoverage',[markdown,{coverageKind:kind}]);probe('assessCurrentEventsCoverage',[markdown]);for(const researchIntent of ['general','docs_reference','current_events']){const facts=output.extractResearchFacts(markdown,4,120);const preview=output.buildResearchPreview(markdown,4,120);const assessment={markdown,facts,preview,intent:researchIntent};probe('assessResearchFetchContent',[assessment]);probe('formatFetchTranscriptContent',[{url:'https://fixture.invalid/page',title:'Fixture API reference',facts,preview,profile:output.deriveResearchCompressionProfile(),intent:researchIntent,authoritative:true,...output.assessResearchFetchContent(assessment),statusReason:output.assessResearchFetchContent(assessment).reason}]);}}
const results=[{title:'Official fixture',url:'https://docs.fixture.invalid/api',snippet:'This is a documented API with parameters and a return value.',authoritative:true},{title:'Second fixture',url:'https://second.invalid/item',snippet:'Independent fixture content.'},{title:'Third fixture',url:'https://third.invalid/item'}];
for(const usage of [{},{contextWindow:64000},{contextWindow:32000}])for(const researchIntent of ['general','docs_reference','current_events'])for(const found of [[],results])probe('formatSearchTranscriptContent',['fixture API parameters',found,output.deriveResearchCompressionProfile(usage),{intent:researchIntent,preferCorroboration:true,originalQuery:'original fixture',searchStrategy:'fixture strategy',followUpQueries:['narrow fixture']}]);
// Rust strings use UTF-8. Make the JSON transport's treatment of split UTF-16
// surrogates explicit without altering the reference algorithm or source.
const result=JSON.parse(JSON.stringify({version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',rows},(_,v)=>typeof v==='string'?v.toWellFormed():v));
const target=path.join(root,'fixtures/tools/research-oracle.json');if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(result,null,2)+'\n');}else assert.deepEqual(result,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',research_cases:rows.length,model_actions:0,network_actions:0}));
