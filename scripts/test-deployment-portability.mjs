/** Isolated real-server acceptance for portable definitions, engine guards, ACL and UI. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'deployment-portable-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`,log=await open(join(work,'server.log'),'w');
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_REDIS_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
const report={work,checks:[]};let browser,page;
const wait=async fn=>{const end=Date.now()+40000;for(;;){if(await fn())return;assert(Date.now()<end,'Timed out');await new Promise(r=>setTimeout(r,100));}};
try{
  await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const context=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:1000},permissions:[]});
  const api=async(path,data)=>{const response=await context.request.fetch(origin+'/api'+path,{method:data?'POST':'GET',headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});assert(response.ok(),`${path}: ${response.status()} ${await response.text()}`);return response.json();};
  await api('/setup',{name:'Portable acceptance',email:'portable@example.invalid',password:'temporary-yaml-acceptance',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const space=await api('/workspaces',{name:'Portable acceptance',slug:'yaml-acceptance'}),root=`/workspaces/${space.id}`;
  const host=await api(root+'/nodes',{name:'YAML target',connection:'runner'});

  const flow=await api(root+'/pipelines',{name:'Portable native build',definition_format:'github-actions',definition:'on: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo CI_BUILD\n',executor_bindings:[{job_id:'build',target_kind:'host',target_ref:host.id,image:'host'}]});
  const manifest={apiVersion:'ferry/v2',kind:'Application',metadata:{name:'portable-web'},services:[{name:'web',image:'nginx:alpine',artifact:{source:'pipeline',pipeline:flow.id,path:'bin/web'}}]};
  const payload=mode=>({name:'Portable '+mode,mode,target_kind:'host',target_ref:host.id,workdir:'/opt/deployments',manifest:JSON.stringify(manifest)});
  const apps={};
  for(const mode of ['native','docker','compose','stack']){
    const app=await api(root+'/deployments',payload(mode));apps[mode]=app;
    const stored=await api(root+'/deployments/'+app.id),definition=JSON.parse(stored.manifest);
    assert.equal(definition.apiVersion,'ferry/v2');assert.equal(definition.services[0].artifact.pipeline,flow.id);assert.equal(definition.services[0].image,'nginx:alpine');
    const rendered=JSON.parse((await api(root+'/deployments/'+app.id+'/renderings',{})).artifact).services.web;
    assert.equal('artifact' in rendered,mode==='native');assert.equal('image' in rendered,mode!=='native');
  }
  const capabilityPath=root+'/deployment-capabilities';
  const activityBefore=(await api(root+'/activity')).total;assert(Number.isInteger(activityBefore));
  assert((await api(capabilityPath,{manifest:JSON.stringify(manifest)})).checks.every(row=>row.supported));
  assert.equal((await api(root+'/activity')).total,activityBefore,'Pure checks do not add mutation audit records');
  report.checks.push('Same v2 definition creates and renders through all four real API paths; both sources persist, only selected source renders; capability checks are read-only');
  const rejected=async(context,path,data,expected)=>{const response=await context.request.post(origin+'/api'+path,{headers:{'X-Ferry-Request':'1'},data});assert.equal(response.status(),expected,await response.text());};
  await rejected(context,root+'/deployments',{...payload('compose'),manifest:JSON.stringify({...manifest,apiVersion:'ferry/v1'})},400);
  const placement={constraints:['node.role == worker','node.labels.zone != zone-a'],maxReplicasPerNode:1};
  const placed=structuredClone(manifest);placed.services[0].placement=placement;
  const checks=(await api(capabilityPath,{manifest:JSON.stringify(placed)})).checks;
  assert.deepEqual(checks.map(row=>[row.mode,row.supported]),[['native',false],['docker',false],['compose',false],['stack',true]]);
  const invalid=structuredClone(placed);invalid.services[0].placement.constraints=['\nnode.role == worker'];
  assert((await api(capabilityPath,{manifest:JSON.stringify(invalid)})).checks.every(row=>!row.supported));
  const stack=await api(root+'/deployments',{...payload('stack'),name:'Placed stack',manifest:JSON.stringify(placed)});
  const stackArtifact=JSON.parse((await api(root+'/deployments/'+stack.id+'/renderings',{})).artifact);
  assert.deepEqual(stackArtifact.services.web.deploy.placement,{constraints:placement.constraints,max_replicas_per_node:1});
  await rejected(context,root+'/deployments',{...payload('compose'),manifest:JSON.stringify(placed)},400);
  const regular=await browser.newContext();
  const user=await regular.request.post(origin+'/api/users',{headers:{'X-Ferry-Request':'1'},data:{name:'Other account',email:'other@example.invalid',password:'temporary-other-account'}});assert(user.ok());
  await rejected(regular,capabilityPath,{manifest:JSON.stringify(manifest)},404);
  const memberSpace=await regular.request.post(origin+'/api/workspaces',{headers:{'X-Ferry-Request':'1'},data:{name:'Other workspace',slug:'other-workspace'}});assert(memberSpace.ok());
  const foreignRoot='/workspaces/'+(await memberSpace.json()).id;
  const foreignHost=await regular.request.post(origin+'/api'+foreignRoot+'/nodes',{headers:{'X-Ferry-Request':'1'},data:{name:'Other target',connection:'runner'}});assert(foreignHost.ok());
  await rejected(regular,foreignRoot+'/deployments',{...payload('compose'),target_ref:(await foreignHost.json()).id},400);
  const anonymous=await browser.newContext();await rejected(anonymous,capabilityPath,{manifest:JSON.stringify(manifest)},401);await anonymous.close();
  report.checks.push('v1 dual-source rejection; Stack placement renders; other engines reject; nonmember and anonymous access denied');
  page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(`${origin}/zh-CN${root}/deployments/${stack.id}/edit`);
  const version=page.getByLabel('应用定义版本',{exact:true});await version.waitFor();assert.equal(await version.inputValue(),'ferry/v2');
  assert.equal(await page.getByRole('textbox',{name:'镜像',exact:true}).inputValue(),'nginx:alpine');
  await page.getByRole('button',{name:'检查部署兼容性',exact:true}).click();
  const results=page.getByRole('region',{name:'检查部署兼容性',exact:true});await results.getByText('配置可渲染',{exact:true}).waitFor();
  assert.equal(await results.getByText('配置不兼容',{exact:true}).count(),3);
  await page.getByRole('textbox',{name:'镜像',exact:true}).fill('nginx:stable');
  await results.getByText('配置可渲染',{exact:true}).waitFor({state:'hidden'});
  assert.equal(JSON.parse((await api(root+'/deployments/'+stack.id)).manifest).services[0].image,'nginx:alpine','Compatibility does not save pending edits');
  await page.getByRole('textbox',{name:'节点约束',exact:true}).fill('node.role == worker');
  await page.getByRole('spinbutton',{name:'每节点副本上限',exact:true}).fill('2');
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await wait(async()=>JSON.parse((await api(root+'/deployments/'+stack.id)).manifest).services[0].placement.maxReplicasPerNode===2);
  const persisted=JSON.parse((await api(root+'/deployments/'+stack.id)).manifest);
  assert.equal(persisted.services[0].image,'nginx:stable');assert.equal(persisted.services[0].artifact.pipeline,flow.id);assert.deepEqual(persisted.services[0].placement.constraints,['node.role == worker']);
  report.checks.push('Real browser edits dual-source and placement; capability matrix shows precise engine failures; editing clears results and saving preserves both sources');
  const translations=JSON.parse(await readFile('src/shared/i18n/locales/resource-domain.json','utf8'));
  for(const locale of ['zh-CN','en-US','zh-TW','ja-JP','ko-KR','ms-MY']){
    await page.setViewportSize({width:390,height:844});
    await page.goto(`${origin}/${locale}${root}/deployments/${stack.id}/edit`);
    await page.getByLabel(translations[locale]['deployment.definitionVersion'],{exact:true}).waitFor();
    await page.getByRole('button',{name:translations[locale]['deployment.checkCompatibility'],exact:true}).click();
    await page.getByText(translations[locale]['deployment.compatible'],{exact:true}).waitFor();
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),locale+' viewport overflow');
    assert(!/resource-domain:|deployment\.portableHelp/.test(await page.locator('main').innerText()));
  }
  report.checks.push('Six locales at390px: dual-source form, placement and capability results without viewport overflow or missing keys');
  assert.deepEqual(errors,[]);report.result='PASS';await page.screenshot({path:join(work,'portable-mobile.png')});
}catch(error){report.result='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(work,'failure.png')});await writeFile(join(work,'failure.html'),await page.content());}throw error;}
finally{
  await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
  await browser?.close();if(server.exitCode===null&&server.signalCode===null){const ended=once(server,'exit');server.kill();await ended;}await log.close();
}
