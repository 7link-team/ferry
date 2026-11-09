/** Isolated profile management and authenticated catalog discovery; no Runner/model calls. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir} from 'node:os';
import {createServer} from 'node:http';
import {chromium} from 'playwright';
const output=resolve(process.env.TEST_OUTPUT || 'data/maintenance/models-credentials-validation');await mkdir(output,{recursive:true});
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
 for(const locale of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY']){
  const catalog=JSON.parse(await readFile(`src/shared/i18n/locales/${locale}.json`,'utf8'));
  const response=await owner.request.get(origin+'/'+locale+'/login',{headers:{'Accept-Language':'ms-MY'}});
  const html=await response.text();assert(html.includes(`<title>${catalog.app.title}</title>`));assert(html.includes(`<html lang="${locale}">`));
 }
 for(const [language,expected] of [['xx-XX','en-US'],['fr-FR,ms-MY;q=0.8','ms-MY'],['zh-CN;q=0,en-US;q=0.5','en-US']]){
  const html=await (await owner.request.get(origin+'/login',{headers:{'Accept-Language':language}})).text();
  const catalog=JSON.parse(await readFile(`src/shared/i18n/locales/${expected}.json`,'utf8'));assert(html.includes(`<title>${catalog.app.title}</title>`));
 }
 const firstPaint=await browser.newContext({javaScriptEnabled:false,locale:'ms-MY'}),firstPage=await firstPaint.newPage();
 await firstPage.goto(origin+'/login');assert.equal(await firstPage.title(),JSON.parse(await readFile('src/shared/i18n/locales/ms-MY.json','utf8')).app.title);await firstPaint.close();
 pass('first HTML title: six URL languages, weighted browser negotiation, unknown English fallback and JavaScript-disabled first paint');
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
 await page.getByRole('combobox',{name:'CLI tool',exact:true}).click();
 for(const runtime of ['Codex','Claude Code','Gemini CLI','Qwen Code','OpenCode','Pi'])assert.equal(await page.getByRole('option',{name:runtime,exact:true}).count(),1);
 await page.getByRole('option',{name:'Claude Code',exact:true}).click();
 await page.getByLabel('Default model',{exact:true}).fill('manual-cli-model');await page.getByRole('button',{name:'Create',exact:true}).click();await page.waitForURL(/\/ai-models$/);
 const profiles=(await admin(root+'/ai-models')).items,cli=profiles.find(v=>v.name==='Native Claude');assert(cli);assert.equal(cli.provider_id,'0');assert.deepEqual(cli.models,['manual-cli-model']);assert.equal(cli.runtime,'claude');
 await member(root+'/ai-models/'+cli.id);await member('/workspaces/'+other.id+'/ai-models/'+cli.id,undefined,'GET',404);
 pass('shared CLI profile form requires no Runner and preserves an explicitly supplied model');
 await page.goto(origin+'/en-US'+root+'/credentials/new?scope=ai');await page.waitForURL(new RegExp('/ai-models/new$'));
 assert.equal(await page.getByLabel('API Key',{exact:true}).count(),1);
 await page.goto(origin+'/en-US/'+ws.id+'/workflow/credentials');await page.waitForURL(new RegExp('/ai-models$'));
 pass('legacy AI credential URLs redirect to canonical model URLs');
 const labels=JSON.parse(await readFile('src/shared/i18n/locales/workflow-domain.json','utf8'));
 await page.goto(origin+'/en-US'+root+'/ai-models/new');await page.getByLabel('Name',{exact:true}).fill('API connection');
 assert.equal(await page.getByLabel('Model connection',{exact:true}).count(),0);
 assert.equal(await page.getByRole('button',{name:'Add model connection',exact:true}).count(),0);
 await page.getByLabel('API Key',{exact:true}).fill('catalog-secret-do-not-expose');
 assert.equal(await page.locator('a[href*="scope=ai"]').count(),0);
 await page.getByLabel(labels.aiBaseURL[0],{exact:true}).fill(mockURL);
 await page.getByLabel(labels.aiModel[0],{exact:true}).fill('model-a');
 const beforeProviders=(await admin(root+'/ai-providers')).items.length;
 await page.getByRole('button',{name:'Load model catalog',exact:true}).click();
 await wait(async()=>(await page.getByLabel('Model catalog (one per line) 1',{exact:true}).inputValue())==='model-a');
 assert.equal((await admin(root+'/ai-providers')).items.length,beforeProviders,'discovery does not persist draft connection');
 await page.getByLabel('Default model',{exact:true}).fill('model-a');
 for(const width of [1440,900,390,320]){
  await page.setViewportSize({width,height:960});
  const name=await page.getByLabel('Name',{exact:true}).boundingBox(),kind=await page.getByRole('combobox',{name:'Connection type',exact:true}).boundingBox();
  const protocol=await page.getByLabel(labels.aiProtocol[0],{exact:true}).boundingBox(),token=await page.getByLabel('API Key',{exact:true}).boundingBox();
  if(width>=768){assert(Math.abs(name.y-kind.y)<2,'short fields share a row');assert(Math.abs(protocol.y-token.y)<2,'protocol and credential share a row');}
  else {assert(kind.y>name.y,'phone stacks short fields');assert(token.y>protocol.y,'phone stacks connection fields');}
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`no page overflow at ${width}`);
  assert.equal(await page.getByLabel('Name',{exact:true}).inputValue(),'API connection');
  await page.screenshot({path:join(output,`model-form-${width}.png`),fullPage:true});
 }
 await page.getByRole('button',{name:'Create',exact:true}).click();await page.waitForURL(/\/ai-models$/);
 const id=(await admin(root+'/ai-models')).items.find(v=>v.name==='API connection').id,first=await admin(root+'/ai-models/'+id);
 assert.equal(first.kind,'api');assert.notEqual(first.provider_id,'0');assert.notEqual(first.connection.credential_id,inlineToken.id);assert.equal(first.connection.base_url,mockURL);assert.deepEqual(first.models,['model-a','model-b']);assert(!JSON.stringify(first).includes('catalog-secret'));
 assert.equal((await admin(root+'/ai-providers')).items.length,beforeProviders+1);
 const second=await admin(root+'/ai-models/'+id,{...first,name:'Updated API connection',state:'disabled'},'PATCH');assert.equal(second.version,2);assert.notEqual(second.revision_id,first.revision_id);await admin(root+'/ai-models/'+id,{...first,name:'Stale'},'PATCH',409);
 await admin('/workspaces/'+other.id+'/ai-models',{...first,provider_id:provider.id,connection:undefined},'POST',404);
 const count=(await admin(root+'/ai-providers')).items.length;
 const rollbackToken=await admin(root+'/credentials',{name:'Rollback token',kind:'token',scope:'ai',secret:'rollback-secret'});
 const invalid={...first,id:undefined,provider_id:'0',connection:{...providerBody,credential_id:rollbackToken.id},default_model:'--invalid'};
 await admin(root+'/ai-models',invalid,'POST',400);
 assert.equal((await admin(root+'/ai-providers')).items.length,count,'invalid model rolls back new connection');
 const updated=await admin(root+'/ai-models/'+id);
 await admin(root+'/ai-models/'+id,{...updated,connection:{...updated.connection,base_url:'invalid'}},'PATCH',400);
 assert.equal((await admin(root+'/ai-models/'+id)).connection.base_url,mockURL);
 await admin('/workspaces/'+other.id+'/ai-models',{...invalid,default_model:'model-a'},'POST',400);
 const reader=await member(root+'/ai-models/'+id);assert.equal(reader.connection,undefined);
 pass('direct connection creation/discovery, atomic rollback, read permission, workspace scope and version conflicts');
 await page.setViewportSize({width:1440,height:960});
 await page.goto(origin+'/en-US'+root+'/ai-models/'+id);
 await page.locator('form.resource-detail').waitFor();assert.equal(await page.locator('form.resource-detail input,form.resource-detail textarea,form.resource-detail select').count(),0);
 await page.getByRole('link',{name:'Edit',exact:true}).click();
 await page.getByLabel(labels.aiBaseURL[0],{exact:true}).waitFor();
 assert.equal(await page.getByLabel(labels.aiBaseURL[0],{exact:true}).inputValue(),mockURL);
 assert.equal(await page.getByLabel('API Key',{exact:true}).inputValue(),'');
 await page.getByLabel('Name',{exact:true}).fill('Edited direct model');
 await page.getByRole('button',{name:'Save',exact:true}).click();await page.waitForURL(/\/ai-models$/);
 assert.equal((await admin(root+'/ai-models/'+id)).name,'Edited direct model');
 pass('read-only text and separate edit route saves model and connection together');
 for(const [i,locale] of ['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'].entries()){
  await page.goto(origin+'/'+locale+root+'/ai-models');const labels=JSON.parse(await readFile('src/shared/i18n/locales/models.json','utf8'));await page.getByRole('link',{name:labels.title[i],exact:true}).first().waitFor();await page.getByRole('table').waitFor();assert(!(await page.locator('body').innerText()).includes('models:'));assert(await page.getByRole('table').locator('.lucide-eye').count()>0);assert(await page.getByRole('table').locator('.lucide-pencil').count()>0);
  assert.equal(await page.title(),JSON.parse(await readFile(`src/shared/i18n/locales/${locale}.json`,'utf8')).app.title);
 }
 for(const locale of ['zh-CN','ms-MY']) for(const width of [900,320]){
  await page.setViewportSize({width,height:960});await page.goto(origin+'/'+locale+root+'/ai-models/'+id+'/edit');
  await page.locator('select').first().waitFor();
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),locale+' form fits '+width);
  await page.screenshot({path:join(output,`model-form-${locale}-${width}.png`),fullPage:true});
 }
 await page.goto(origin+'/en-US'+root+'/ai-models');await page.getByRole('table').waitFor();
 await page.screenshot({path:join(output,'models-desktop.png'),fullPage:true});await page.setViewportSize({width:390,height:844});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));await page.screenshot({path:join(output,'models-mobile.png'),fullPage:true});assert.deepEqual(errors,[]);
 pass('six languages and mobile model resource list');report.status='PASS';
}catch(error){report.status='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(output,'failure.png'),fullPage:true}).catch(()=>{});await writeFile(join(output,'failure.txt'),await page.locator('body').innerText()).catch(()=>{});}throw error;}
finally{await browser?.close();if(server&&server.exitCode===null){const done=once(server,'exit');server.kill();await done;}await log.close();await new Promise(r=>mock.close(r));await writeFile(join(output,'result.json'),JSON.stringify(report,null,2)+'\n');}
