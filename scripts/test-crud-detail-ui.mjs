/** Real-server acceptance: read-only detail, URL-backed editors and safe return. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'crud-detail-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`,log=await open(join(work,'server.log'),'w');
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_REDIS_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
const report={work,checks:[]};let browser,page;
const wait=async fn=>{const end=Date.now()+40000;for(;;){if(await fn())return;assert(Date.now()<end,'Timed out');await new Promise(r=>setTimeout(r,100));}};
try {
  await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const context=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:900}});
  const api=async(path,data,method)=>{const response=await context.request.fetch(origin+'/api'+path,{method:method||(data?'POST':'GET'),headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});assert(response.ok(),`${path}: ${response.status()} ${await response.text()}`);return response.json();};
  await api('/setup',{name:'CRUD acceptance',email:'crud@example.invalid',password:'temporary-crud-acceptance',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const space=await api('/workspaces',{name:'CRUD acceptance',slug:'crud-acceptance'}),root=`/workspaces/${space.id}`;
  const connection=await api(root+'/code-connections',{name:'Repository-'.padEnd(60,'x'),provider:'git',url:'https://example.invalid/example/app.git'});
  const host=await api(root+'/nodes',{name:'Runner acceptance',connection:'runner'});
  const monitorPath=root+'/code-connection-monitors/'+connection.id;
  let monitor=await api(monitorPath);
  await api(monitorPath,{version:monitor.version,mode:'poll',runner_id:host.id,enabled:false,all_repositories:false,repositories:'https://example.invalid/example/app.git',events:['push'],notification_channels:[]},'PATCH');
  const insight=await api(root+'/code-insights',{name:'Insight-'.padEnd(60,'x'),enabled:false,update_mode:'scheduled',schedule:'0 */4 * * *',timezone:'UTC',full_history:false,initial_days:90,reconcile_minutes:1440,repositories:[{connection_id:connection.id,repository:'https://example.invalid/example/app.git'}],aliases:[{name:'Contributor',email:'contributor@example.invalid',nickname:''}]});
  page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
  const views=[['settings',root+'/code-insights/'+insight.id+'?tab=settings&at=30'],['authors',root+'/code-insights/'+insight.id+'?tab=authors&at=30'],['monitoring',root+'/code-connections/'+connection.id+'?tab=monitoring&at=30']];
  for(const width of [1440,900,390,320])for(const [tab,path] of views){
    await page.setViewportSize({width,height:900});await page.goto(origin+'/zh-CN'+path);
    const form=page.locator('form.resource-detail');await form.waitFor();
    assert.equal(await form.locator('input:not([aria-hidden=true]):not([type=hidden]),textarea,select,[role=combobox]').count(),0,tab+' read-only controls');
    assert.equal(await form.locator('button[type=submit]').count(),0);
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),tab+' view overflow '+width);
    await page.screenshot({path:join(work,`${tab}-view-${width}.png`),fullPage:true});
    await form.getByRole('link',{name:'编辑',exact:true}).click();
    await page.locator('button[type=submit]').last().waitFor();
    assert(new URL(page.url()).pathname.endsWith('/edit'));assert.equal(new URL(page.url()).searchParams.get('tab'),tab);
    assert.equal(await page.getByRole('dialog').count(),0);
    await page.reload();await page.locator('button[type=submit]').last().waitFor();
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),tab+' edit overflow '+width);
    if(width<1200)for(const button of await page.locator('.crud-config-page [data-slot=button]:visible').all()) {
      const box=await button.boundingBox();assert(box.height>=44&&box.width>=44,tab+' touch target '+width);
    }
    await page.screenshot({path:join(work,`${tab}-edit-${width}.png`),fullPage:true});
    await page.getByRole('link',{name:'取消',exact:true}).click();
    await page.locator('form.resource-detail').waitFor();assert.equal(new URL(page.url()).pathname+new URL(page.url()).search,'/zh-CN'+path);
    report.checks.push(`${tab}: ${width}px read-only, URL edit, reload, cancel restores tab and at, no overflow`);
  }
  // A stale writer stays on the editor with its draft; the server refuses the transition.
  await page.goto(origin+'/zh-CN'+views[0][1]);await page.locator('form.resource-detail').getByRole('link',{name:'编辑',exact:true}).click();
  await page.locator('input[name=name]').fill('Draft retained after conflict');
  const endpoint=root+'/code-insights/'+insight.id,beforeConflict=await api(endpoint);
  await api(endpoint,{...beforeConflict,name:'Concurrent change'},'PATCH');
  const [conflict]=await Promise.all([page.waitForResponse(r=>r.url().endsWith(endpoint)&&r.request().method()==='PATCH'),page.getByRole('button',{name:'保存',exact:true}).click()]);
  assert.equal(conflict.status(),409);assert(new URL(page.url()).pathname.endsWith('/edit'));
  assert.equal(await page.locator('input[name=name]').inputValue(),'Draft retained after conflict');
  assert((await page.locator('form > p.text-destructive').innerText()).length>0);
  await page.getByRole('link',{name:'取消',exact:true}).click();
  report.checks.push('Real optimistic conflict: 409, draft and edit URL retained, error visible, cancel returns safely');
  // A real write must return to the detail, preserve unrelated fields and use a current version.
  for(const [tab,path] of views){
    await page.goto(origin+'/zh-CN'+path);await page.locator('form.resource-detail').getByRole('link',{name:'编辑',exact:true}).click();
    await page.locator('button[type=submit]').last().waitFor();
    const endpoint=tab==='monitoring'?monitorPath:root+'/code-insights/'+insight.id;
    const before=await api(endpoint);
    const response=page.waitForResponse(r=>r.url().endsWith(endpoint)&&r.request().method()==='PATCH');
    await page.getByRole('button',{name:'保存',exact:true}).click();assert((await response).ok());
    await page.locator('form.resource-detail').waitFor();assert.equal(new URL(page.url()).pathname+new URL(page.url()).search,'/zh-CN'+path);
    const after=await api(endpoint);assert(after.version>before.version);assert.equal(after.enabled,false);
    if(tab==='monitoring'){assert.equal(after.runner_id,before.runner_id);assert.deepEqual(after.events,before.events);}
    else {assert.deepEqual(after.aliases,before.aliases);assert.deepEqual(after.repositories,before.repositories);}
    report.checks.push(tab+': real PATCH with optimistic version, safe return, unrelated data preserved');
  }
  for(const locale of ['en-US','ms-MY'])for(const width of [1440,900,390])for(const [tab,path] of views){
    await page.setViewportSize({width,height:900});await page.goto(origin+'/'+locale+path);await page.locator('form.resource-detail').waitFor();
    assert.equal(await page.locator('form.resource-detail').locator('input:not([aria-hidden=true]):not([type=hidden]),textarea,select').count(),0);
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
    await page.screenshot({path:join(work,`${locale}-${tab}-${width}.png`),fullPage:true});
  }
  await page.setViewportSize({width:900,height:900});await page.goto(origin+'/zh-CN'+views[0][1]);await page.locator('form.resource-detail').waitFor();
  await page.addStyleTag({content:'html{font-size:24px}'});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
  assert.deepEqual(errors,[]);report.checks.push('EN/MS three dimensions and 150% font: no document overflow or JS errors');
  report.result='PASS';console.log(JSON.stringify(report));
} catch(error){report.result='FAIL';report.error=String(error.stack||error);if(page){report.url=page.url();report.body=await page.locator('body').innerText();await page.screenshot({path:join(work,'failure.png'),fullPage:true});}console.error(report.error);process.exitCode=1;}
finally{await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));if(browser)await browser.close();server.kill();await log.close();}
