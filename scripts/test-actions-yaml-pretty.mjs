/** Real Monaco: compact YAML loads legibly, editing stays literal, formatting is explicit. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {createServer} from 'node:net';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {chromium} from 'playwright';
import {parse} from 'yaml';

const work=await mkdtemp(join(tmpdir(),'actions-yaml-pretty-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`,log=await open(join(work,'server.log'),'w');
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_REDIS_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
const report={work,checks:[]};let browser;
const wait=async fn=>{const end=Date.now()+40000;for(;;){if(await fn())return;assert(Date.now()<end,'Timed out');await new Promise(r=>setTimeout(r,100));}};
try{
  await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const context=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:1000},permissions:['clipboard-read','clipboard-write']});
  const api=async(path,data)=>{const response=await context.request.fetch(origin+'/api'+path,{method:data?'POST':'GET',headers:{'X-Ferry-Request':'1'},...(data?{data}:{})});assert(response.ok(),await response.text());return response.json();};
  await api('/setup',{name:'YAML acceptance',email:'yaml@example.invalid',password:'temporary-yaml-acceptance',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const space=await api('/workspaces',{name:'YAML acceptance',slug:'yaml-acceptance'}),root=`/workspaces/${space.id}`;
  const host=await api(root+'/nodes',{name:'YAML target',connection:'runner'});
  const compact='on: workflow_dispatch\njobs: { build: { runs-on: self-hosted, steps: [ { run: "echo \\\"{\\\\\\\"ok\\\\\\\":true}\\\"\\necho done" } ] } }\n';
  const flow=await api(root+'/pipelines',{name:'Compact YAML acceptance',definition_format:'github-actions',definition:compact,executor_bindings:[{job_id:'build',target_kind:'host',target_ref:host.id,image:'host'}]});
  const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(`${origin}/zh-CN${root}/pipelines/${flow.id}/edit?editor=yaml`);
  const editor=page.locator('.monaco-editor .view-lines');await editor.waitFor();
  const read=async()=>{await editor.click();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.press('ControlOrMeta+C');return(await page.evaluate(()=>navigator.clipboard.readText())).replace(/\r\n/g,'\n');};
  const set=async text=>{await editor.click();await page.keyboard.press('ControlOrMeta+A');await page.evaluate(value=>navigator.clipboard.writeText(value),text);await page.keyboard.press('ControlOrMeta+V');};
  let displayed=await read();assert.match(displayed,/jobs:\n  build:\n/);assert.deepEqual(parse(displayed),parse(compact));
  assert.equal((await api(root+'/pipelines/'+flow.id)).definition,compact,'Opening does not rewrite saved YAML');
  report.checks.push('Compact saved YAML opens as indented YAML; scripts retain exact JSON/text; no data mutation on view');
  const edited='on: workflow_dispatch\njobs: { build: { runs-on: self-hosted, steps: [ { run: echo CI_BUILD } ] } }\n';
  await set(edited);await wait(async()=> (await read()).includes('jobs: {'));
  assert.equal(await read(),edited,'Typing does not reformat each keystroke');
  await page.getByRole('button',{name:'格式化 YAML',exact:true}).click();
  await wait(async()=> (await read()).includes('jobs:\n  build:\n'));
  displayed=await read();assert.deepEqual(parse(displayed),parse(edited));
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await wait(async()=> (await api(root+'/pipelines/'+flow.id)).definition===displayed);
  report.checks.push('Explicit format keeps semantics, generates block YAML, and persists the displayed source');
  await page.goto(`${origin}/zh-CN${root}/pipelines/${flow.id}/edit?editor=yaml`);await editor.waitFor();
  await set('jobs: [');await page.getByRole('button',{name:'格式化 YAML',exact:true}).click();
  await page.getByRole('alert').filter({hasText:/Flow sequence/}).first().waitFor();
  assert.equal((await read()).trim(),'jobs: [','Invalid source is not discarded');
  assert.equal((await api(root+'/pipelines/'+flow.id)).definition,displayed);
  report.checks.push('Invalid YAML has inline feedback, keeps typed text, and never changes saved data');
  assert.deepEqual(errors,[]);report.result='PASS';
  await page.screenshot({path:join(work,'validation.png')});
  await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
}finally{
  await browser?.close();if(server.exitCode===null&&server.signalCode===null){const ended=once(server,'exit');server.kill();await ended;}await log.close();
}
