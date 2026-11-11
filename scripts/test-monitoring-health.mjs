/** Isolated HTTP/TCP, shared notification and browser acceptance. Never uses an existing DB. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createServer} from 'node:http';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'monitoring-health-'));
let healthy=true;const notices=[];
const receiver=createServer(async(req,res)=>{
  if(req.method==='POST'){let body='';for await(const chunk of req)body+=chunk;notices.push(JSON.parse(body));res.writeHead(204).end();}
  else res.writeHead(healthy?200:503,{'content-type':'text/plain'}).end(healthy?'healthy':'failed');
});
await new Promise(r=>receiver.listen(0,'127.0.0.1',r));const target=`http://127.0.0.1:${receiver.address().port}`;
const portHolder=createServer();await new Promise(r=>portHolder.listen(0,'127.0.0.1',r));const port=portHolder.address().port;await new Promise(r=>portHolder.close(r));
const origin=`http://127.0.0.1:${port}`;
const log=await open(join(work,'server.log'),'w');
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_REDIS_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const binary=resolve(process.env.FERRY_BINARY||'target/debug/ferry-server'+(process.platform==='win32'?'.exe':''));
const server=spawn(binary,['--desktop-control'],{env,windowsHide:true,stdio:['pipe',log.fd,log.fd]});
const report={work,checks:[],screenshots:[],limitations:['Linux Native/Docker/Compose/Swarm live engines are not available on this test host.','Figma and physical mobile keyboard are not available.']};
let browser;
const wait=async(fn,label,timeout=45000)=>{const end=Date.now()+timeout;for(;;){if(await fn())return;assert(Date.now()<end,label);await new Promise(r=>setTimeout(r,200));}};
const pass=label=>{report.checks.push(label);console.log('PASS',label);};
try{
  await wait(async()=>{assert.equal(server.exitCode,null,'Isolated server exited');try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}},'server startup');
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const context=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:1000}});
  const anonymous=await browser.newContext();
  const request=async(c,path,data,method=data?'POST':'GET',expected=200)=>{
    const response=await c.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data!==undefined?{data}:{})});
    assert.equal(response.status(),expected,`${method} ${path}: ${await response.text()}`);
    return response.status()===204?null:response.json();
  };
  const api=(path,data,method,expected)=>request(context,path,data,method,expected);
  await api('/setup',{name:'Monitoring acceptance',email:'monitoring@example.invalid',password:'temporary-monitoring-acceptance',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const space=await api('/workspaces',{name:'Monitoring acceptance',slug:'monitoring-acceptance'});
  const other=await api('/workspaces',{name:'Other',slug:'monitoring-other'});
  const root=`/workspaces/${space.id}`;
  await request(anonymous,root+'/health-checks',undefined,'GET',401);
  const checkBody={name:'Entry check',description:'',kind:'http',role:'entry',address:target,enabled:false};
  const created=await api(root+'/health-checks',checkBody,'POST',201);
  let check=await api(root+'/health-checks/'+created.id);
  for(const [key,value] of Object.entries({interval_seconds:30,timeout_seconds:5,failure_threshold:3,recovery_threshold:2,stale_seconds:90,retention_days:7}))assert.equal(check[key],value,key);
  assert.equal(check.state,'paused');
  await api(`/workspaces/${other.id}/health-checks/${created.id}`,undefined,'GET',404);
  await api(root+'/health-checks',{...checkBody,name:'Invalid',interval_seconds:0},'POST',400);
  pass('Default configurable parameters, paused creation, anonymous denial, validation and workspace isolation');
  const host=await api(root+'/nodes',{name:'Unconnected runtime target',connection:'runner'},'POST',201);
  const app=await api(root+'/deployments',{name:'Health example',mode:'docker',target_kind:'host',target_ref:host.id,workdir:'/opt/deployments',manifest:JSON.stringify({apiVersion:'ferry/v1',kind:'Application',metadata:{name:'health-example'},services:[{name:'web',image:'busybox:stable'}]})},'POST',201);
  await api(root+'/health-checks',{...checkBody,name:'Cross-space binding',deployment_id:app.id},'POST',201);
  await api(`/workspaces/${other.id}/health-checks`,{...checkBody,name:'Invalid binding',deployment_id:app.id},'POST',404);
  const application=await api(root+'/application-health/'+app.id);assert.equal(application.health,'unknown');assert.equal(application.unmonitored_hosts,1);
  await api(root+'/health-checks/'+created.id,{...check,enabled:true,deployment_id:app.id,interval_seconds:5,timeout_seconds:1,failure_threshold:2,recovery_threshold:2,stale_seconds:8},'PATCH');
  await wait(async()=>{check=await api(root+'/health-checks/'+created.id);return check.state==='up';},'first real HTTP observation');
  assert.equal(check.deployment_name,'Health example');assert.equal(check.result.observation.status_code,200);
  await api(root+'/health-checks/'+created.id,{...check,version:1},'PATCH',409);
  pass('Real HTTP success, deployed-app binding, scoped display names and optimistic concurrency');
  const tcp=await api(root+'/health-checks',{...checkBody,name:'TCP check',kind:'tcp',address:`127.0.0.1:${receiver.address().port}`,enabled:true,interval_seconds:5,timeout_seconds:1,stale_seconds:8},'POST',201);
  await wait(async()=>(await api(root+'/health-checks/'+tcp.id)).state==='up','real TCP observation');
  const channel=await api(root+'/notification-channels',{name:'Local test receiver',provider:'Webhook',webhook:target,language:'zh-CN',enabled:true},'POST',201);
  const rule=await api(root+'/alert-rules',{name:'Health failure',target_kind:'health-check',target_ref:created.id,metric:'health-failed',enabled:true,notification_channel_id:channel.id},'POST',201);
  healthy=false;
  await wait(async()=>(await api(root+'/alert-events?rule_id='+rule.id)).items.some(event=>event.state==='firing'),'confirmed failure event');
  await wait(async()=>notices.length===1,'shared trigger notification');
  check=await api(root+'/health-checks/'+created.id);assert.equal(check.state,'down');
  healthy=true;
  await wait(async()=>(await api(root+'/alert-events?rule_id='+rule.id)).items.some(event=>event.state==='recovered'),'confirmed recovery event');
  await wait(async()=>notices.length===2,'shared recovery notification');
  assert.equal((await api(root+'/alert-events?rule_id='+rule.id)).total,1);
  assert((await api(root+'/health-observations?check_id='+created.id)).total>=4);
  await api(root+'/health-checks/'+created.id,undefined,'DELETE',409);
  pass('Real TCP, consecutive HTTP failure/recovery, one durable event, unified local webhook delivery and reference protection');

  const runtime=await api(root+'/health-checks',{name:'Runtime unavailable',kind:'runtime',role:'service',deployment_id:app.id,host_id:host.id,enabled:true,interval_seconds:5,timeout_seconds:1,stale_seconds:8},'POST',201);
  await wait(async()=>{const row=await api(root+'/health-checks/'+runtime.id);return row.checked_at>0&&row.state==='unknown';},'unavailable runtime unknown');
  assert.equal((await api(root+'/application-health/'+app.id)).health,'unknown');
  pass('An undeployed or unobservable runtime stays unknown and cannot make application health green');

  const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.setViewportSize({width:320,height:1000});
  await page.goto(`${origin}/zh-CN/workspaces/${space.id}/health-checks/new`);
  await page.locator('input[name="name"]').fill('Browser-created check '+ 'x'.repeat(98));
  await page.locator('input[name="address"]').fill(target);
  const saved=page.waitForResponse(response=>response.url().endsWith(root+'/health-checks')&&response.request().method()==='POST');
  await page.locator('button[type="submit"]').click();
  assert.equal((await saved).status(),201);
  const browserCreated=(await api(root+'/health-checks?q=Browser-created')).items[0];
  assert(browserCreated);assert.equal(browserCreated.enabled,false);
  assert.equal(browserCreated.interval_seconds,30);
  pass('Shared browser form creates a paused check with editable default parameters');
  for(const width of [1440,900,390,320]){
    await page.setViewportSize({width,height:1000});
    for(const [name,path] of [['overview','application-health'],['checks','health-checks'],['check-detail','health-checks/'+created.id],['application-detail','application-health/'+app.id],['new','health-checks/new']]){
      await page.goto(`${origin}/zh-CN/workspaces/${space.id}/${path}`);await page.waitForLoadState('networkidle');
      assert(!await page.getByRole('alert').filter({hasText:/404|monitoring\./}).count(),`${name} rendered an error`);
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`page overflow: ${name}/${width}`);
      const image=join(work,`${name}-${width}.png`);await page.screenshot({path:image,fullPage:true});report.screenshots.push(image);
    }
  }
  for(const locale of ['en-US','ms-MY']){
    await page.goto(`${origin}/${locale}/workspaces/${space.id}/health-checks/new`);await page.waitForLoadState('networkidle');
    assert(!await page.locator('body').innerText().then(text=>/monitoring:|monitoring\./.test(text)),'untranslated monitoring key');
    const image=join(work,`new-${locale}-320.png`);await page.screenshot({path:image,fullPage:true});report.screenshots.push(image);
  }
  assert.deepEqual(errors,[]);pass('Real pages in Desktop/Pad/Phone/320px, English/Malay forms, no page overflow or browser exceptions');
  await api(root+'/alert-rules/'+rule.id,undefined,'DELETE',204);
  await api(root+'/health-checks/'+created.id,undefined,'DELETE',204);
  await api(root+'/health-checks/'+created.id,undefined,'GET',404);
  pass('Removing the referencing rule permits audited deletion of the check and its observation history');
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=error.stack;throw error;}
finally{
  if(browser)await browser.close();server.stdin?.end();
  await Promise.race([new Promise(r=>server.once('exit',r)),new Promise(r=>setTimeout(r,10000))]);
  if(server.exitCode===null)server.kill();receiver.closeAllConnections();await new Promise(r=>receiver.close(r));
  await writeFile(join(work,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify({work,result:report.result,checks:report.checks.length}));await log.close();
}
