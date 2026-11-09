/** Real isolated account preference persistence and browser regression. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {createServer} from 'node:http';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {chromium} from 'playwright';

const output=resolve(process.env.TEST_OUTPUT || `data/maintenance/account-preferences-${Date.now()}`);
await mkdir(output,{recursive:true});
const work=await mkdtemp(join(tmpdir(),'account-preferences-'));
const reservation=createServer();
await new Promise(done=>reservation.listen(0,'127.0.0.1',done));
const origin=`http://127.0.0.1:${reservation.address().port}`;
await new Promise(done=>reservation.close(done));
const env={...process.env,FERRY_BIND:new URL(origin).host,FERRY_DATA_DIR:work,FERRY_SERVER_MODE:'false'};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const log=await open(join(output,'server.log'),'a');
const report={work,checks:[]};
const pass=name=>{report.checks.push(name);console.log('PASS',name);};
let server,browser;
const wait=async(fn)=>{const deadline=Date.now()+60000;while(Date.now()<deadline){if(await fn())return;await new Promise(done=>setTimeout(done,100));}throw Error('Timed out');};
try {
  server=spawn(resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${process.platform === 'win32' ? '.exe' : ''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
  await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
  browser=await chromium.launch({headless:true});
  const context=await browser.newContext();
  const request=async(ctx,path,data,method=data?'POST':'GET',status=200)=>{
    const response=await ctx.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1'},...(data?{data}:{})});
    assert.equal(response.status(),status,`${method} ${path}: ${await response.text()}`);
    return response.json();
  };
  await request(context,'/setup',{name:'Preference owner',email:'owner@example.invalid',password:'preference-fixture-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const anonymous=await browser.newContext();
  await request(anonymous,'/me/preferences',undefined,'GET',401);
  await request(anonymous,'/me/preferences',{locale:'en-US'},'PATCH',401);
  pass('anonymous read/write rejected');
  const settings=await request(context,'/platform/settings/1');
  await request(context,'/platform/settings/1',{version:settings.version,registration_enabled:true},'PATCH');
  await request(anonymous,'/users',{name:'Other user',email:'other@example.invalid',password:'other-fixture-password'});
  assert.deepEqual(await request(anonymous,'/me/preferences'),{theme:null,locale:null});
  await request(context,'/me/preferences',{user_id:'other',locale:'ja-JP'},'PATCH',422);
  await request(context,'/me/preferences',{locale:'xx'},'PATCH',400);
  pass('account selection and invalid locale rejected');
  const theme={palette:'custom',mode:'dark',customLight:{},customDark:{'--primary':'#123456'}};
  await request(context,'/me/preferences',{theme,locale:'en-US'},'PATCH');
  assert.deepEqual(await request(anonymous,'/me/preferences'),{theme:null,locale:null});
  pass('preferences isolated between two actual accounts');
  const fresh=await browser.newContext();
  await request(fresh,'/sessions',{email:'owner@example.invalid',password:'preference-fixture-password'});
  assert.deepEqual(await request(fresh,'/me/preferences'),{theme,locale:'en-US'});
  pass('fresh browser session restores persisted custom theme and locale');
  const page=await fresh.newPage();
  const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(origin+'/zh-CN/platform/workspaces');
  await page.waitForURL('**/en-US/platform/workspaces');
  await wait(()=>page.evaluate(()=>document.documentElement.dataset.theme==='custom' && document.documentElement.classList.contains('dark') && document.documentElement.style.getPropertyValue('--primary')==='#123456'));
  await page.getByRole('button',{name:'Theme',exact:true}).click();
  await page.getByRole('menuitem',{name:'Eye Care',exact:true}).click();
  await wait(async()=>(await request(fresh,'/me/preferences')).theme.palette==='eyeCare');
  await page.getByRole('button',{name:'Language',exact:true}).click();
  await page.getByRole('menuitemradio',{name:'日本語',exact:true}).click();
  await wait(async()=>(await request(fresh,'/me/preferences')).locale==='ja-JP');
  pass('real theme/language switchers write cloud preferences');
  for(const width of [1440,900,390,320]){
    await page.setViewportSize({width,height:900});
    await page.screenshot({path:join(output,`preferences-${width}.png`),fullPage:true});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`overflow at ${width}`);
  }
  assert.deepEqual(errors,[]);
  pass('four viewport widths without page overflow or browser exceptions');
  const secondPage=await context.newPage();
  await secondPage.goto(origin+'/en-US/platform/workspaces');
  await secondPage.waitForURL('**/ja-JP/platform/workspaces');
  assert.equal(await secondPage.locator('html').getAttribute('data-theme'),'eyeCare');
  pass('another browser context receives latest settings at next load');
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error.stack||error);throw error;}
finally{
  await writeFile(join(output,'result.json'),JSON.stringify(report,null,2));
  await browser?.close();server?.kill();await log.close();
}
