/** Real HTTP authorization, cross-workspace audit and storage-path browser acceptance. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const work=await mkdtemp(join(tmpdir(),'platform-activity-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`;
const log=await open(join(work,'server.log'),'w');
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE','FERRY_ARTIFACT_STORE'])delete env[key];
const server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,stdio:['ignore',log.fd,log.fd],windowsHide:true});
let browser, page;
const report={work,checks:[]};
try {
  for(let i=0;;i++) {
    assert.equal(server.exitCode,null);
    try {if((await fetch(origin+'/api/status')).ok)break;}catch{}
    assert(i<200,'startup timed out');await new Promise(r=>setTimeout(r,100));
  }
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const admin=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:950}});
  const regular=await browser.newContext();const anonymous=await browser.newContext();
  const api=async(context,path,data,method=data?'POST':'GET',status)=>{
    const r=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});
    if(status===undefined)assert(r.ok(),`${method} ${path}: ${await r.text()}`);
    else assert.equal(r.status(),status,`${method} ${path}: ${await r.text()}`);
    return r.status()===204?undefined:r.json();
  };
  await api(admin,'/setup',{email:'admin@example.invalid',password:'temporary-admin-password',name:'Audit administrator',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  await api(regular,'/users',{email:'member@example.invalid',password:'temporary-member-password',name:'Workspace operator'});
  await api(regular,'/sessions',{email:'member@example.invalid',password:'temporary-member-password'});
  const one=await api(regular,'/workspaces',{name:'Audit one',slug:'audit-one'});
  const two=await api(admin,'/workspaces',{name:'Audit two',slug:'audit-two'});
  await api(regular,`/workspaces/${one.id}/nodes`,{name:'Owned node',connection:'runner'});
  await api(admin,`/workspaces/${two.id}/nodes`,{name:'Other node',connection:'runner'});
  const endpoint='/platform/monitor/activity';
  await api(anonymous,endpoint,undefined,'GET',401);
  await api(regular,endpoint,undefined,'GET',403);
  await api(regular,endpoint+'?workspace_id='+one.id,undefined,'GET',403);
  const all=await api(admin,endpoint);
  assert(all.items.some(r=>r.workspace_id==='0'));
  assert(all.items.some(r=>r.workspace_id===one.id&&r.workspace_name==='Audit one'&&r.actor_name==='Workspace operator'));
  assert(all.items.some(r=>r.workspace_id===two.id&&r.workspace_name==='Audit two'));
  assert(all.items.every(r=>typeof r.id==='string'&&typeof r.resource_id==='string'));
  assert.equal((await api(admin,endpoint)).total,all.total,'reading audit must not add audit records');
  const platform=await api(admin,endpoint+'?scope=platform');assert(platform.items.length&&platform.items.every(r=>r.workspace_id==='0'));
  const spaces=await api(admin,endpoint+'?scope=workspace');assert(spaces.items.length&&spaces.items.every(r=>r.workspace_id!=='0'));
  assert.equal(platform.total+spaces.total,all.total);
  const filtered=await api(admin,endpoint+`?workspace_id=${one.id}&q=Workspace%20operator&action=create&resource=hosts`);
  assert(filtered.items.length&&filtered.items.every(r=>r.workspace_id===one.id&&r.actor_name==='Workspace operator'&&r.action==='create'&&r.resource==='hosts'));
  const tenant=await api(regular,`/workspaces/${one.id}/activity`);assert(!tenant.items.some(r=>r.workspace_id==='0'||r.resource_id===two.id));
  const page1=await api(admin,endpoint+'?page_size=1&sort=created_at');
  const page2=await api(admin,endpoint+'?page_size=1&page=2&sort=created_at');assert.equal(page1.items.length,1);assert.notEqual(page1.items[0].id,page2.items[0].id);
  assert.equal((await api(admin,endpoint+'?page_size=999')).page_size,200);
  for(const query of ['scope=wrong','workspace_id=invalid','sort=actor_name','unknown=1'])await api(admin,endpoint+'?'+query,undefined,'GET',400);
  report.checks.push('Admin-only all-scope audit, anonymous/member rejection, tenant isolation, actor/workspace names, combined filters, stable pagination, size cap, invalid queries, reads do not write');
  const settings=await api(admin,'/platform/settings/1');assert.equal(settings.storage_local_path,resolve(work));
  const p=await admin.newPage();const errors=[];p.on('pageerror',e=>errors.push(String(e)));
  page=p;
  const expectedAsset=(await readFile('dist/index.html','utf8')).match(/index-[\w-]+\.js/)[0];
  assert((await (await fetch(origin)).text()).includes(expectedAsset),'server must embed the current production UI');
  await p.goto(origin+'/zh-CN/platform/monitor/activity');
  await p.getByRole('columnheader',{name:'范围 / 工作空间',exact:true}).waitFor();
  await p.getByRole('cell',{name:'Audit one',exact:true}).first().waitFor();
  assert.equal(await p.getByRole('button',{name:'新增',exact:true}).count(),0);
  assert(await p.locator('nav a[aria-current="page"]').filter({hasText:'操作记录'}).count());
  await p.screenshot({path:join(work,'activity.png')});
  await p.setViewportSize({width:390,height:844});assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await p.screenshot({path:join(work,'activity-mobile.png')});
  await p.goto(origin+'/zh-CN/platform/settings/1/edit');
  await p.getByRole('button',{name:'文件存储',exact:true}).click();
  await p.getByText(settings.storage_local_path,{exact:true}).waitFor();
  assert.equal(await p.locator('input[value="'+settings.storage_local_path.replaceAll('\\','\\\\')+'"]').count(),0,'path is read-only display');
  await p.screenshot({path:join(work,'storage-path.png')});
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  assert.deepEqual(errors,[]);
  report.checks.push('Real list/navigation, read-only absolute storage path, 1440/390px layouts, no browser errors');
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(work,'failure.png')});await writeFile(join(work,'failure.html'),await page.content());}throw error;}
finally {
  await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));
  console.log('Platform activity acceptance:',work,report.result);
  await browser?.close();server.kill();await log.close();
}
