/** SMS configuration/UI acceptance only. No commercial SMS request is sent. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {createServer} from 'node:http';
import {chromium} from 'playwright';
const evidence=resolve('data/maintenance/sms-providers');await mkdir(evidence,{recursive:true});
const work=await mkdtemp(join(tmpdir(),'notification-sms-'));
const probe=createServer();await new Promise(r=>probe.listen(0,'127.0.0.1',r));
const bind=`127.0.0.1:${probe.address().port}`,origin=`http://${bind}`;await new Promise(r=>probe.close(r));
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:bind};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE','FERRY_ARTIFACT_STORE'])delete env[key];
const log=await open(join(work,'server.log'),'a'),report={work,checks:[],screenshots:[],commercial_sms_sent:false};
const wait=async fn=>{for(let n=0;n<200;n++){if(await fn())return;await new Promise(r=>setTimeout(r,200));}throw Error('Acceptance timeout');};
let server,browser;
try {
 server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
 await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
 browser=await chromium.launch({channel:'chrome',headless:true});const c=await browser.newContext(),p=await c.newPage(),errors=[];
 p.on('pageerror',e=>errors.push(e.message));
 const api=async(path,data,method=data?'POST':'GET',status)=>{const r=await c.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'en-US'},...(data?{data}:{})});assert(status?r.status()===status:r.ok(),`${method} ${path} ${r.status()} ${await r.text()}`);return r.status()===204?null:r.json();};
 await api('/setup',{name:'SMS fixture owner',email:'sms-owner@example.invalid',password:'temporary-sms-fixture-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
 const space=await api('/workspaces',{name:'SMS acceptance',slug:'sms-acceptance'}),other=await api('/workspaces',{name:'Other space',slug:'sms-other'});
 const root=`/workspaces/${space.id}`,channels=root+'/notification-channels';
 const base={name:'SMS fixture',provider:'Aliyun SMS',enabled:false,sms_access_key:'fixtureAccess',sms_secret_key:'fixtureSecret',sms_sign_name:'Fixture',sms_template_code:'123456',sms_parameter_names:['code'],sms_region:'ap-guangzhou',sms_app_id:'1400000000',recipients:['+8613800000000']};
 const ids=[];
 for(const provider of ['Aliyun SMS','Tencent SMS']){
  const created=await api(channels,{...base,provider,name:provider});ids.push(created.id);
  const view=await api(`${channels}/${created.id}`);assert.equal(view.has_sms_secret,true);assert(!JSON.stringify(view).includes('fixtureSecret'));
  await api(`${channels}/${created.id}`,{...base,provider,name:provider,sms_secret_key:'',version:view.version},'PATCH');
  const preview=await api(`${channels}/${created.id}/previews`,{client_id:'preview',variables:{code:'123456'}});
  assert.equal(preview.parameters.code,'123456');assert.equal(preview.sms_template_code,'123456');assert.deepEqual(preview.to,base.recipients);
  await api(`/workspaces/${other.id}/notification-channels/${created.id}`,undefined,'GET',404);
  await api(`${channels}/${created.id}/deliveries`,{client_id:'disabled-no-send',variables:{code:'123456'}},'POST',409);
 }
 assert.equal((await api(root+'/notification-deliveries')).total,0);
 report.checks.push('Both SMS providers: encrypted secret projection, empty-secret update, template preview, cross-space rejection and disabled send; zero delivery records');
 const sms=await api(channels+'?provider='+encodeURIComponent('Aliyun SMS,Tencent SMS'));assert.equal(sms.total,2);
 assert.equal((await api(channels+'?provider=SMTP')).total,0);
 for(const width of [1440,900,390,320]){
  await p.setViewportSize({width,height:960});await p.goto(`${origin}/zh-CN${channels}?scope=sms`);
  const sidebar=p.getByRole('navigation',{name:'维护',exact:true});await sidebar.waitFor();
  const group=sidebar.getByRole('group',{name:'通知',exact:true});assert.deepEqual(await group.locator('a').allTextContents(),['邮件','短信','第三方平台']);
  const button=group.getByRole('button',{name:'通知',exact:true});await button.click();assert.equal(await button.getAttribute('aria-expanded'),'false');await button.click();
  assert.equal(await sidebar.locator('[aria-current=page]').textContent(),'短信');
  await p.getByRole('cell',{name:'Aliyun SMS',exact:true}).first().waitFor();
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
  await p.goto(`${origin}/zh-CN${channels}/${ids[1]}/edit?scope=sms`);
  const name=p.locator('input[name=name]');await name.waitFor();await wait(async()=>(await name.inputValue())==='Tencent SMS');
  await p.getByRole('combobox',{name:'地域',exact:true}).waitFor();
  assert.equal(await p.locator('input[name=sms_secret_key]').inputValue(),'');
  assert.equal(await p.locator('input[name=sms_app_id]').inputValue(),'1400000000');
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
  const screenshot=`sms-${width}.png`;await p.screenshot({path:join(evidence,screenshot),fullPage:true});report.screenshots.push(screenshot);
 }
 report.checks.push('SMS route, independently collapsible notification group, provider fields and secret masking at 1440/900/390/320');
 await p.goto(`${origin}/zh-CN${channels}/new?scope=sms`);await p.locator('input[name=sms_access_key]').waitFor();
 assert(await p.getByRole('combobox').allTextContents().then(values=>values.some(v=>v.includes('Aliyun SMS'))));
 for(const locale of ['en-US','ms-MY']){
  await p.goto(`${origin}/${locale}${channels}/${ids[0]}/edit?scope=sms`);await p.locator('input[name=sms_access_key]').waitFor();
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
 }
 assert.deepEqual(errors,[]);report.result='PASS';
}catch(e){report.result='FAIL';report.error=String(e.stack||e);throw e;}
finally{await browser?.close();if(server&&server.exitCode===null){server.kill();await new Promise(r=>server.once('exit',r));}await log.close();await writeFile(join(evidence,'acceptance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));}
