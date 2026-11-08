/** Isolated profile management and authenticated catalog discovery; no Runner/model calls. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {createServer} from 'node:http';
import {chromium} from 'playwright';
const output=resolve('data/maintenance/models-validation');await mkdir(output,{recursive:true});
const work=await mkdtemp(join(tmpdir(),'model-profiles-'));const report={work,checks:[]};
const pass=s=>{report.checks.push(s);console.log('PASS',s);};
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function wait(fn,seconds=40){const end=Date.now()+seconds*1000;while(Date.now()<end){const r=await fn();if(r)return r;await delay(150);}throw Error('Acceptance timeout');}
let discoveryMode='success',seen=[];
const mock=createServer((req,res)=>{
 seen.push({url:req.url,authorization:req.headers.authorization,key:req.headers['x-api-key'],version:req.headers['anthropic-version']});
 if(discoveryMode==='redirect'){res.writeHead(302,{Location:'/must-not-follow'});res.end();return;}
 if(discoveryMode==='error'){res.writeHead(401);res.end('sensitive upstream diagnostic');return;}
 if(discoveryMode==='oversized'){res.end(JSON.stringify({data:[{id:'x'.repeat(1024*1024)}]}));return;}
 if(discoveryMode==='empty'){res.end(JSON.stringify({data:[]}));return;}
 res.setHeader('Content-Type','application/json');res.end(JSON.stringify({data:[{id:'model-b'},{id:'model-a'},{id:'model-a'}],has_more:false}));
});
await new Promise(r=>mock.listen(0,'127.0.0.1',r));const mockURL=`http://127.0.0.1:${mock.address().port}/v1`;
const reservation=createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${reservation.address().port}`;await new Promise(r=>reservation.close(r));
const env={...process.env,FERRY_BIND:new URL(origin).host,FERRY_DATA_DIR:join(work,'server')};for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];await mkdir(env.FERRY_DATA_DIR);
const log=await open(join(work,'server.log'),'a');let server,browser,page;
try{
 server=spawn(resolve(process.env.FERRY_BINARY||'target/debug/ferry-server.exe'),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
 await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
 browser=await chromium.launch({channel:'chrome',headless:true});const owner=await browser.newContext({viewport:{width:1366,height:900}}),peer=await browser.newContext();
 const api=ctx=>async(path,data,method=data?'POST':'GET',expected)=>{const r=await ctx.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1'},...(data?{data}:{})});assert(expected?r.status()===expected:r.ok(),`${method} ${path}: ${r.status()} ${await r.text()}`);return r.json();};
 const admin=api(owner),member=api(peer);
 await admin('/setup',{name:'Owner',email:'owner@example.invalid',password:'model-owner-password',token:(await readFile(join(env.FERRY_DATA_DIR,'setup-token'),'utf8')).trim()});
 const ws=await admin('/workspaces',{name:'Model acceptance',slug:'models'}),other=await admin('/workspaces',{name:'Other',slug:'other'}),root='/workspaces/'+ws.id;
 const credential=await admin(root+'/credentials',{name:'Catalog token',kind:'token',scope:'ai',secret:'catalog-secret-do-not-expose'});
 const providerBody={name:'OpenAI-compatible',credential_id:credential.id,base_url:mockURL,protocol:'openai',currency:'USD',timezone:'UTC',monthly_limit_micros:0,qps:1,concurrency:1,prices:[{model:'model-a',operation:'ai.llm.chat',input_micros:0,output_micros:0,unit_size:1}]};
 const anthropicToken=await admin(root+'/credentials',{name:'Anthropic token',kind:'token',scope:'ai',secret:'catalog-secret-do-not-expose'}),inlineToken=await admin(root+'/credentials',{name:'Inline token',kind:'token',scope:'ai',secret:'catalog-secret-do-not-expose'});
 const provider=await admin(root+'/ai-providers',providerBody),anthropic=await admin(root+'/ai-providers',{...providerBody,credential_id:anthropicToken.id,name:'Anthropic',protocol:'anthropic'});
 const discovered=await admin(root+'/ai-models/model-discoveries',{provider_id:provider.id});assert.deepEqual(discovered.models,['model-a','model-b']);assert.equal(seen.at(-1).authorization,'Bearer catalog-secret-do-not-expose');assert.equal(seen.at(-1).url,'/v1/models');
 await admin(root+'/ai-models/model-discoveries',{provider_id:anthropic.id});assert.equal(seen.at(-1).key,'catalog-secret-do-not-expose');assert.equal(seen.at(-1).version,'2023-06-01');assert.equal(seen.at(-1).authorization,undefined);
 for(const mode of ['error','redirect','oversized','empty']){discoveryMode=mode;const start=seen.length;const e=await admin(root+'/ai-models/model-discoveries',{provider_id:provider.id},'POST',502);assert(!JSON.stringify(e).includes('sensitive upstream'));assert(!JSON.stringify(e).includes('catalog-secret'));if(mode==='redirect')assert.equal(seen.length,start+1);}
 discoveryMode='success';pass('OpenAI/Anthropic authenticated discovery, bounded responses, sanitized errors and no redirects');
 await admin('/workspaces/'+other.id+'/ai-models/model-discoveries',{provider_id:provider.id},'POST',404);
 await member('/users',{name:'Reader',email:'reader@example.invalid',password:'model-reader-password'});await member('/workspace-memberships',{code:(await admin(root+'/members/invitation')).code});
 const people=(await admin(root+'/members')).items,person=people.find(p=>p.email==='reader@example.invalid');const role=await admin(root+'/roles',{name:'Profile reader',permissions:['ai-models.read']});await admin(root+'/members/'+person.id,{role_id:role.id,version:person.version},'PATCH');
 await member(root+'/ai-models/model-discoveries',{provider_id:provider.id},'POST',403);await member(root+'/ai-models',{name:'Denied',kind:'cli',runtime:'codex',state:'enabled'},'POST',403);
 pass('workspace provider boundaries and independent profile read/write/credential-use permissions');
 page=await owner.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto(origin+'/en-US'+root+'/ai-models/new');await page.getByLabel('Name',{exact:true}).fill('Native Claude');
 await page.getByRole('combobox',{name:'Connection type',exact:true}).click();await page.getByRole('option',{name:'CLI',exact:true}).click();
 await page.getByRole('combobox',{name:'CLI tool',exact:true}).click();await page.getByRole('option',{name:'Claude Code',exact:true}).click();
 await page.getByLabel('Default model',{exact:true}).fill('manual-cli-model');await page.getByRole('button',{name:'Create',exact:true}).click();await page.waitForURL(/\/ai-models$/);
 const profiles=(await admin(root+'/ai-models')).items,cli=profiles.find(v=>v.name==='Native Claude');assert(cli);assert.equal(cli.provider_id,'0');assert.deepEqual(cli.models,['manual-cli-model']);assert.equal(cli.runtime,'claude');
 await member(root+'/ai-models/'+cli.id);await member('/workspaces/'+other.id+'/ai-models/'+cli.id,undefined,'GET',404);
 pass('shared CLI profile form requires no Runner and preserves an explicitly supplied model');
 await page.goto(origin+'/en-US'+root+'/ai-models/new');await page.getByLabel('Name',{exact:true}).fill('API connection');await page.getByLabel('Model connection',{exact:true}).selectOption(provider.id);
 await page.getByRole('button',{name:'Load model catalog',exact:true}).click();await wait(async()=>(await page.getByLabel('Model catalog (one per line)',{exact:true}).inputValue()).includes('model-a'));
 await page.getByLabel('Default model',{exact:true}).fill('model-a');await page.getByRole('button',{name:'Create',exact:true}).click();await page.waitForURL(/\/ai-models$/);
 const id=(await admin(root+'/ai-models')).items.find(v=>v.name==='API connection').id,first=await admin(root+'/ai-models/'+id);assert.equal(first.kind,'api');assert.equal(first.provider_id,provider.id);assert.deepEqual(first.models,['model-a','model-b']);assert(!JSON.stringify(first).includes('catalog-secret'));
 const second=await admin(root+'/ai-models/'+id,{...first,name:'Updated API connection',state:'disabled'},'PATCH');assert.equal(second.version,2);assert.notEqual(second.revision_id,first.revision_id);await admin(root+'/ai-models/'+id,{...first,name:'Stale'},'PATCH',409);
 await admin('/workspaces/'+other.id+'/ai-models',{...first,provider_id:provider.id},'POST',404);
 pass('shared API form loads models, stores provider references only and freezes immutable revisions with optimistic updates');
 await page.goto(origin+'/en-US'+root+'/ai-models/new');await page.getByLabel('Name',{exact:true}).fill('Inline provider profile');await page.getByLabel('Default model',{exact:true}).fill('model-a');
 await page.getByRole('button',{name:'Add model connection',exact:true}).click();assert(page.url().endsWith('/new'));const providerEditor=page.getByRole('region',{name:'Add model connection',exact:true});
 await providerEditor.getByLabel('API token credential',{exact:true}).selectOption(inlineToken.id);
 const labels=JSON.parse(await readFile('src/shared/i18n/locales/workflow-domain.json','utf8'));
 await providerEditor.getByLabel('Name',{exact:true}).fill('Inline connection');await providerEditor.getByLabel(labels.aiBaseURL[0],{exact:true}).fill(mockURL);await providerEditor.getByLabel(labels.aiModel[0],{exact:true}).fill('model-a');
 await providerEditor.getByRole('button',{name:'Save',exact:true}).click();await wait(async()=>(await admin(root+'/ai-providers')).items.some(p=>p.name==='Inline connection'));assert(page.url().endsWith('/new'));
 await page.getByRole('button',{name:'Create',exact:true}).click();await page.waitForURL(/\/ai-models$/);pass('reused provider editor saves inside the shared profile form without submitting its parent');
 for(const [i,locale] of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'].entries()){
  await page.goto(origin+'/'+locale+root+'/ai-models');const labels=JSON.parse(await readFile('src/shared/i18n/locales/models.json','utf8'));await page.getByRole('link',{name:labels.title[i],exact:true}).first().waitFor();await page.getByRole('table').waitFor();assert(!(await page.locator('body').innerText()).includes('models:'));assert(await page.getByRole('table').locator('.lucide-eye').count()>0);assert(await page.getByRole('table').locator('.lucide-pencil').count()>0);
 }
 await page.screenshot({path:join(output,'models-desktop.png'),fullPage:true});await page.setViewportSize({width:390,height:844});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));await page.screenshot({path:join(output,'models-mobile.png'),fullPage:true});assert.deepEqual(errors,[]);
 pass('six languages and mobile model resource list');report.status='PASS';
}catch(error){report.status='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(output,'failure.png'),fullPage:true}).catch(()=>{});await writeFile(join(output,'failure.txt'),await page.locator('body').innerText()).catch(()=>{});}throw error;}
finally{await browser?.close();if(server&&server.exitCode===null){const done=once(server,'exit');server.kill();await done;}await log.close();await new Promise(r=>mock.close(r));await writeFile(join(output,'result.json'),JSON.stringify(report,null,2)+'\n');}
