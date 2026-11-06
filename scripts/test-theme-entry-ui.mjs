/** Real-server coverage of appearance controls across application layouts. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createServer} from 'node:net';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'theme-entry-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`;
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const log=await open(join(work,'server.log'),'w');
const server=spawn(resolve(`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,stdio:['ignore',log.fd,log.fd],windowsHide:true});
const report={work,checks:[]};let browser,page;
try{
  for(let i=0;;i++){
    assert.equal(server.exitCode,null);
    try{if((await fetch(origin+'/api/status')).ok)break;}catch{}
    assert(i<200);await new Promise(r=>setTimeout(r,100));
  }
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const context=await browser.newContext({locale:'zh-CN',viewport:{width:1280,height:900}});
  page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(String(e)));
  const api=async(path,data)=>{
    const response=await context.request.fetch(origin+'/api'+path,{method:data?'POST':'GET',headers:{'X-Ferry-Request':'1'},...(data?{data}:{})});
    assert(response.ok(),await response.text());return response.json();
  };
  const theme=()=>page.getByRole('button',{name:'主题',exact:true});
  const check=async(name,path)=>{
    const ready=()=>name==='platform-forbidden'?page.getByText('当前账号没有平台管理权限。',{exact:true}).waitFor():page.locator('header,h1').first().waitFor();
    await page.goto(origin+'/zh-CN'+path);await ready();await theme().waitFor();
    assert.equal(await theme().count(),1,name+' must have exactly one theme entry');
    assert.equal(await page.getByRole('button',{name:'语言',exact:true}).count(),1);
    await theme().click();await page.getByRole('menuitem',{name:'深色',exact:true}).click();
    assert(await page.locator('html').evaluate(el=>el.classList.contains('dark')),name);
    await page.reload();await ready();await theme().waitFor();assert(await page.locator('html').evaluate(el=>el.classList.contains('dark')));
    await page.setViewportSize({width:390,height:844});const box=await theme().boundingBox();assert(box&&box.x>=0&&box.x+box.width<=390);
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:join(work,name+'.png')});
    await theme().click();await page.getByRole('menuitem',{name:'浅色',exact:true}).click();
    assert(!await page.locator('html').evaluate(el=>el.classList.contains('dark')));
    await page.setViewportSize({width:1280,height:900});report.checks.push(name);
  };
  await check('initialization','/login');
  await api('/setup',{name:'Theme admin',email:'admin@example.invalid',password:'temporary-theme-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  await context.clearCookies();await check('login','/login');
  await api('/sessions',{email:'admin@example.invalid',password:'temporary-theme-password'});
  const space=await api('/workspaces',{name:'Theme workspace',slug:'theme-workspace'});
  await check('platform','/platform/workspaces');
  await check('account','/account');
  await check('workspace',`/workspaces/${space.id}/hosts`);
  await check('actions-editor',`/workspaces/${space.id}/pipelines/new`);
  await check('node-editor',`/workspaces/${space.id}/workflows/new`);
  const member=await browser.newContext({locale:'zh-CN'});
  const registered=await member.request.post(origin+'/api/users',{headers:{'X-Ferry-Request':'1'},data:{email:'member@example.invalid',name:'Theme member',password:'temporary-member-password'}});assert(registered.ok(),await registered.text());
  await page.close();page=await member.newPage();page.on('pageerror',e=>errors.push(String(e)));
  await check('workspace-onboarding','/workspaces');
  await check('platform-forbidden','/platform/workspaces');
  assert.deepEqual(errors,[]);report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error);if(page)await page.screenshot({path:join(work,'failure.png')});throw error;
}finally{
  await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log('Theme entry acceptance:',work,report.result);
  await browser?.close();if(server.exitCode===null){const closed=new Promise(r=>server.once('exit',r));server.kill();await closed;}await log.close();
}
