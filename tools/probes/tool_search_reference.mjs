import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';import {fileURLToPath,pathToFileURL} from 'node:url';
await import('../offline_guard.mjs');const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../..');
const privateRoot=fs.mkdtempSync(path.join(root,'.local/p09-search-'));process.env.AG1_CODING_AGENT_DIR=path.join(privateRoot,'agent');process.env.PATH='/usr/bin:/bin';process.env.RIPGREP_CONFIG_PATH='/dev/null';
const base=path.join(root,'reference/ag1-executable/packages/coding-agent/dist/core/tools');
const {createGrepToolDefinition}=await import(pathToFileURL(path.join(base,'grep.js'))),{createFindToolDefinition}=await import(pathToFileURL(path.join(base,'find.js')));
const files={
 'src/thing.ts':'// needle comment\nexport function needle() {\n  return 1;\n}\n',
 'src/thing.test.ts':'test("needle", () => {});\n',
 'scripts/compare.mjs':'export const needle = 1;\n',
 'README.md':'The needle is described here.\n',
 'lib/core/tools/tool.rs':'fn needle() {}\n',
 'src/normalize.ts':'export function normalizeCurrentEventsQuery(query) { return query; } // current-events normalization\n',
 'reports/research-output.md':'needle report\n',
 'src/unicode.ts':'const β = "🦙 needle";\n',
 'single.txt':'before\nunique_match\nafter\n',
 'mixed.txt':'one\rtwo\nneedle\n',
 'crlf.txt':'one\r\nneedle\r\nthree\r\n',
 'long.txt':`before\nneedle ${'β🦙'.repeat(300)}\nafter\n`,
 'empty.txt':'', '.hidden.txt':'needle hidden\n', '.gitignore':'ignored.txt\n', 'ignored.txt':'needle ignored\n',
};
const grep=[{}, {path:'src'}, {path:'single.txt',pattern:'unique_match',context:1}, {path:'src/thing.ts',limit:1}, {path:'crlf.txt',context:1}, {path:'mixed.txt',context:1}, {path:'long.txt'}, {pattern:'NEEDLE',ignoreCase:true}, {glob:'*.ts'}, {pattern:'current-events'}, {pattern:'normalization'}, {path:'single.txt',pattern:'missing'}, {path:'empty.txt',pattern:'missing'}, {pattern:'foo|bar',literal:true}, {pattern:'needle.*',literal:true}, {pattern:'[',literal:false}].map(args=>({pattern:'needle',...args}));
const find=[{pattern:'*.ts'},{pattern:'*thing*'},{pattern:'*.mjs'},{pattern:'src'},{pattern:'single.txt',limit:1},{pattern:'*.txt',path:'src'},{pattern:'*missing*'},{pattern:'*archive*compare*runner*'},{pattern:'*.ts',path:'src'},{pattern:'*.txt'}];
const cwd=path.join(privateRoot,'workspace');fs.mkdirSync(cwd);fs.mkdirSync(path.join(cwd,'.git'));
for(const [name,content]of Object.entries(files)){fs.mkdirSync(path.dirname(path.join(cwd,name)),{recursive:true});fs.writeFileSync(path.join(cwd,name),content);}
const results={grep:[],find:[]};
try{for(const [name,inputs,definition]of [['grep',grep,createGrepToolDefinition(cwd)],['find',find,createFindToolDefinition(cwd)]]){
 for(const [i,input]of inputs.entries()){
  let output;try{output={result:await definition.execute(`${name}-${i}`,input)};}catch(e){output={error:e.message};}
  results[name].push({input,output:JSON.parse(JSON.stringify(output).replaceAll(cwd,'/fixture'))});
 }
}}finally{fs.rmSync(privateRoot,{recursive:true,force:true});}
const out={version:1,source_commit:'e30f0b4b238ee5618928bcd17b4571eb046e7038',model_actions:0,network_actions:0,files,...results};const target=path.join(root,'fixtures/tools/search-oracle.json');
if(process.argv.includes('--capture')){assert(!fs.existsSync(target));fs.writeFileSync(target,JSON.stringify(out,null,2)+'\n');}else assert.deepEqual(out,JSON.parse(fs.readFileSync(target,'utf8')));
console.log(JSON.stringify({status:'passed',grep_cases:grep.length,find_cases:find.length,model_actions:0,network_actions:0}));
