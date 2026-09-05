// Reproduce the storage boundary used by AgentSessionRuntime.fork.
import { mkdtempSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
const root=resolve('research/scratch/ag1-reference');
const {SessionManager}=await import(pathToFileURL(join(root,'packages/coding-agent/dist/core/session-manager.js')));
const {ArtifactStore}=await import(pathToFileURL(join(root,'packages/coding-agent/dist/core/artifact-store.js')));
const dir=mkdtempSync(resolve('research/scratch/fork-'));
const sessions=join(dir,'sessions');
const manager=SessionManager.create(dir,sessions);
const parentId=manager.getSessionId();
const parentStore=new ArtifactStore(join(sessions,'artifacts',parentId),parentId);
const artifact=parentStore.putText('Required historical evidence',{producer:{type:'system',name:'fork-probe'}});
const entry=manager.appendCustomEntry('context_capsule_probe',{artifactId:artifact.id});
const forkPath=manager.createBranchedSession(entry);
const fork=SessionManager.open(forkPath,sessions);
const forkStore=new ArtifactStore(join(sessions,'artifacts',fork.getSessionId()),fork.getSessionId());
let childError;try{forkStore.read(artifact.id);}catch(e){childError=e.message;}
const result={parentId,childId:fork.getSessionId(),artifactId:artifact.id,referenceSurvives:fork.getBranch().some(e=>e.type==='custom'&&e.data?.artifactId===artifact.id),parentReadable:parentStore.read(artifact.id).text,childReadable:!childError,childError,scope:'Real SessionManager fork and ArtifactStore paths matching AgentSession constructor; facade path separately traced, no model calls.'};
writeFileSync('research/evidence/ag1-artifact-fork-probe.json',JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify(result,null,2));
