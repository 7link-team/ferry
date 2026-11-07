/** Real browser acceptance for fixed dialog chrome and shared resource forms. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'dialog-layout-'));
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
  const errors=[];page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));
  const node=(id,type,params)=>({id,name:id,type,type_version:1,params,settings:{},position:[0,0]});
  const graph={engine:'ferry-graph',schema:1,nodes:[node('start','core.trigger.manual',{input_schema:{type:'object',properties:Object.fromEntries(Array.from({length:32},(_,i)=>['field_'+i,{type:'string'}]))}})],connections:[]};
  const workflow=await api(root+'/workflows',{name:'Long dialog acceptance',definition:JSON.stringify(graph)});
  const assertChrome=async(label)=>{
    const dialog=page.getByRole('dialog').last();await dialog.waitFor();
    const title=dialog.getByRole('heading').first(),footer=dialog.locator('.dialog-footer').last(),body=dialog.locator('.dialog-body').first();
    await footer.waitFor();
    const before={title:await title.boundingBox(),footer:await footer.boundingBox()};
    const metrics=await body.evaluate(el=>({height:el.clientHeight,scroll:el.scrollHeight}));
    assert(metrics.height>0&&metrics.scroll>metrics.height+30,label+' must really overflow the body');
    await body.evaluate(el=>el.scrollTop=el.scrollHeight);
    const after={title:await title.boundingBox(),footer:await footer.boundingBox()};
    assert(Math.abs(before.title.y-after.title.y)<1,label+' title moved');
    assert(Math.abs(before.footer.y-after.footer.y)<1,label+' footer moved');
    assert(after.title.y>=0&&after.footer.y+after.footer.height<=page.viewportSize().height,label+' chrome outside viewport');
    assert.equal(await dialog.evaluate(el=>el.scrollTop),0,label+' outer surface scrolled');
    assert(await body.evaluate(el=>el.scrollTop>0),label+' body did not scroll');
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),label+' document overflow');
    await page.screenshot({path:join(work,label+'.png'),fullPage:true});
    report.checks.push(label+': real long body scrolls; title/footer positions unchanged and visible');
  };
  for(const width of [1440,390]){
    await page.setViewportSize({width,height:560});
    await page.goto(origin+'/zh-CN'+root+'/workflows/'+workflow.id+'/edit');
    await page.getByRole('button',{name:'保存并运行',exact:true}).click();
    await assertChrome('workflow-run-'+width);
    const dialog=page.getByRole('dialog');
    await dialog.getByRole('textbox',{name:'field_31',exact:true}).fill('last-field');
    await dialog.getByRole('button',{name:'取消',exact:true}).click();
    assert.equal(await page.getByRole('dialog').count(),0);
  }
  // The shared action modal is exercised through a real build pipeline run.
  const host=await api(root+'/nodes',{name:'Alert target',connection:'runner'});
  const actionYaml='name: Parameter dialog\non:\n  workflow_dispatch:\n    inputs:\n'+Array.from({length:24},(_,i)=>`      param_${i}:\n        type: string\n        description: Parameter ${i}\n`).join('')+'jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo CI_DIALOG\n';
  const pipeline=await api(root+'/pipelines',{name:'Parameter dialog',target_kind:'host',target_ref:host.id,definition_format:'github-actions',definition:actionYaml});
  await page.goto(origin+'/zh-CN'+root+'/pipelines/'+pipeline.id);
  const run=page.getByRole('button',{name:'触发',exact:true});
  {
    await run.click();const dialog=page.getByRole('dialog');await dialog.waitFor();
    await assertChrome('shared-action-390');
    await dialog.getByRole('button',{name:'取消',exact:true}).click();
  }
  // Resource forms navigate to the same CRUD page used by their lists.
  await page.setViewportSize({width:1440,height:800});
  const parent=await api(root+'/node-groups',{name:'Parent group',description:'',parent_id:''});
  await page.goto(origin+'/zh-CN'+root+'/nodes');
  await page.getByRole('button',{name:'新建分组',exact:true}).click();
  await page.waitForURL('**/node-groups/new?**');assert.equal(await page.getByRole('dialog').count(),0);
  await page.getByRole('textbox').first().fill('Created through shared form');
  await page.getByRole('button',{name:'创建',exact:true}).click();
  await page.waitForURL('**/nodes');
  assert((await api(root+'/node-groups')).items.some(x=>x.name==='Created through shared form'));
  await page.getByRole('button',{name:'操作 · Parent group',exact:true}).click();
  await page.getByRole('menuitem',{name:'新建子分组',exact:true}).click();
  await page.waitForURL(url=>url.pathname.endsWith('/node-groups/new')&&url.searchParams.get('parent_id')===parent.id);
  const parentChoice=page.locator('select[name=parent_id]');
  if(await parentChoice.count())assert.equal(await parentChoice.inputValue(),parent.id);
  await page.getByRole('textbox').first().fill('Child through shared form');
  await page.getByRole('button',{name:'创建',exact:true}).click();await page.waitForURL('**/nodes');
  assert((await api(root+'/node-groups')).items.some(x=>x.name==='Child through shared form'&&x.parent_id===parent.id));
  report.checks.push('Group create/add-child use shared CRUD route, persist through real API, retain parent, no popup');
  await page.goto(origin+'/zh-CN'+root+'/nodes/'+host.id+'?tab=alerts');
  await page.getByRole('link',{name:'新增告警规则',exact:true}).click();
  await page.waitForURL(url=>url.pathname.endsWith('/alert-rules/new')&&url.searchParams.get('target_ref')===host.id);
  assert.equal(await page.getByRole('dialog').count(),0);
  await wait(async()=> (await page.locator('form').innerText()).includes('Alert target'));
  assert(await page.locator('form select').evaluateAll((els,id)=>els.some(el=>el.value===id),host.id));
  report.checks.push('Host alert create uses ordinary CRUD route with host preselected');
  assert.deepEqual(errors,[]);
  report.result='PASS';console.log(JSON.stringify(report));
} catch(error){report.result='FAIL';report.error=String(error.stack||error);if(page){report.url=page.url();report.body=await page.locator('body').innerText();await page.screenshot({path:join(work,'failure.png'),fullPage:true});}console.error(report.error);process.exitCode=1;}
finally{await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));if(browser)await browser.close();server.kill();await log.close();}
