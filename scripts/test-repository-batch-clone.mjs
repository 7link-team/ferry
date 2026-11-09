/** Isolated batch clone UI + HTTP Git + real Agent/Actions regression. No user data or external writes. */
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import {createServer} from 'node:http';
import {mkdtemp,mkdir,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {chromium} from 'playwright';

const output=resolve(process.env.TEST_OUTPUT||'data/maintenance/repository-batch-clone-'+Date.now());
await mkdir(output,{recursive:true});
const work=await mkdtemp(join(tmpdir(),'repository-connection-reuse-'));
const report={work,checks:[],screenshots:[]};
const pass=name=>{report.checks.push(name);console.log('PASS',name);};
const wait=async(fn,timeout=45000)=>{const deadline=Date.now()+timeout;while(Date.now()<deadline){const result=await fn();if(result)return result;await new Promise(r=>setTimeout(r,150));}throw Error('Timed out');};
const requests=[];
let gitFixtureRoot;
const upstream=createServer(async(req,res)=>{
  if(req.url.startsWith('/team/') || req.url.startsWith('/fixture.git/')){
    if(!gitFixtureRoot){res.writeHead(404);res.end();return;}
    try{
      const url=new URL(req.url,'http://fixture'),chunks=[];for await(const chunk of req)chunks.push(chunk);
      const raw=execFileSync('git',['http-backend'],{windowsHide:true,input:Buffer.concat(chunks),env:{...process.env,GIT_PROJECT_ROOT:gitFixtureRoot,GIT_HTTP_EXPORT_ALL:'1',REQUEST_METHOD:req.method,PATH_INFO:url.pathname,QUERY_STRING:url.search.slice(1),CONTENT_TYPE:req.headers['content-type']??'',REMOTE_ADDR:'127.0.0.1'}});
      const crlf=raw.indexOf('\r\n\r\n'),split=crlf>=0?crlf:raw.indexOf('\n\n');assert(split>=0);
      for(const line of raw.subarray(0,split).toString().split(/\r?\n/)){const colon=line.indexOf(':');if(colon<0)continue;const name=line.slice(0,colon),value=line.slice(colon+1).trim();if(name.toLowerCase()==='status')res.statusCode=Number(value.split(' ')[0]);else res.setHeader(name,value);}
      res.end(raw.subarray(split+(crlf>=0?4:2)));
    }catch{res.writeHead(500);res.end();}return;
  }
  requests.push({url:req.url,token:req.headers['private-token']});
  res.setHeader('Content-Type','application/json');
  const project={id:42,name:'app',path_with_namespace:'team/app',http_url_to_repo:`${remote}/team/app.git`,default_branch:'main',web_url:`${remote}/team/app`};
  if(req.url.split('?')[0]==='/api/v4/user')res.end(JSON.stringify({id:1,username:'acceptance'}));
  else if(req.url.split('?')[0]==='/api/v4/projects')res.end(JSON.stringify([project,{...project,id:43,name:'lib',path_with_namespace:'team/sub/lib',http_url_to_repo:`${remote}/team/sub/lib.git`}]));
  else res.end(JSON.stringify(project));
});
await new Promise(r=>upstream.listen(0,'127.0.0.1',r));
const remote=`http://127.0.0.1:${upstream.address().port}`;
const reservation=createServer();await new Promise(r=>reservation.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${reservation.address().port}`;await new Promise(r=>reservation.close(r));
const env={...process.env,FERRY_BIND:new URL(origin).host,FERRY_DATA_DIR:work,FERRY_SANDBOX_PATH:resolve('target/debug/ferry-sandbox.exe'),ACTIONS_WORKER_DIR:resolve('target/debug/actions-workers-preview-current')};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const log=await open(join(work,'server.log'),'a');
let server,browser,runner;
const node=(id,type,params)=>({id,name:id,type,type_version:1,params,position:[id==='start'?0:320,160]});
const graph=(type,params)=>({engine:'ferry-graph',schema:1,nodes:[node('start','core.trigger.manual',{input_schema:{type:'object'}}),node('repository',type,params)],connections:[{from:{node:'start',port:'main',index:0},to:{node:'repository',port:'main',index:0}}]});
try{
 server=spawn(resolve(process.env.FERRY_BINARY||'target/debug/ferry-server.exe'),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
 await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});
 browser=await chromium.launch({headless:true});const context=await browser.newContext({viewport:{width:1440,height:1000}});
 const api=async(path,data,method=data?'POST':'GET',status)=>{
  const response=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});
  const text=await response.text();assert(status?response.status()===status:response.ok(),`${method} ${path}: ${response.status()} ${text}`);return text?JSON.parse(text):undefined;
 };
 await api('/setup',{name:'Owner',email:'owner@example.invalid',password:'batch-clone-fixture-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
 const workspace=await api('/workspaces',{name:'Batch clone acceptance',slug:'batch-clone'});const root='/workspaces/'+workspace.id;
 const credential=await api(root+'/credentials',{name:'Fixture token',kind:'token',scope:'build',secret:'PRIVATE_ACCEPTANCE_TOKEN'});
 const connection=await api(root+'/code-connections',{name:'GitLab clone group',provider:'gitlab',credential_id:credential.id,base_url:remote,api_url:remote+'/api/v4',organization:'',description:''});
 const repositories=[{url:remote+'/team/app.git',name:'app',full_name:'team/app'},{url:remote+'/team/sub/lib.git',name:'lib',full_name:'team/sub/lib'}];
 gitFixtureRoot=join(work,'git-http');await mkdir(gitFixtureRoot,{recursive:true});
 const source=join(work,'source');await mkdir(source);
 const git=(...args)=>execFileSync('git',args,{windowsHide:true,stdio:'pipe'});
 git('init',source);await writeFile(join(source,'.gitattributes'),'* text eol=lf\n');await writeFile(join(source,'proof.txt'),'BATCH_CLONE_CONTENT\n');git('-C',source,'add','.');git('-C',source,'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','batch fixture');
 for(const path of ['team/app.git','team/sub/lib.git']){await mkdir(join(gitFixtureRoot,path,'..'),{recursive:true});git('clone','--bare',source,join(gitFixtureRoot,path));}
 const host=await api(root+'/nodes',{name:'Batch clone Agent',connection:'runner'});const token=await api(root+'/runner-tokens',{host_id:host.id});
 runner=spawn(resolve('target/debug/ferry-runner-cli-catalog.exe'),['--server',origin,'--token',token.token,'--state-dir',join(work,'agent')],{env:{...process.env,PATH:process.env.PATH+';C:\\Program Files\\Git\\bin'},windowsHide:true,stdio:['ignore',log.fd,log.fd]});
 await wait(async()=>{assert.equal(runner.exitCode,null);return(await api(root+'/nodes/'+host.id)).state==='online';});
 const directory=join(work,'clone-output').replaceAll('\\','/');
 const definition=graph('integration.git.repository.clone',{connection_id:connection.id,url:repositories[0].url,directory,depth:1});
 definition.settings={execution:{runtime:'host',target_kind:'host',target_ref:host.id}};
 let flow=await api(root+'/workflows',{name:'Batch group clone',definition:JSON.stringify(definition)});
 const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
 for(const width of [1440,900,390,320]){
  await page.setViewportSize({width,height:960});await page.goto(origin+'/zh-CN'+root+'/workflows/'+flow.id+'/edit');
  await page.locator('.react-flow__node[data-id="repository"]').dblclick();
  const dialog=page.getByRole('dialog');await dialog.getByLabel('批量克隆仓库 / 分组',{exact:true}).check();
  await dialog.getByText('选择分组（可多选）',{exact:true}).click();await dialog.getByLabel('team · 2',{exact:true}).check();
  assert(await dialog.getByLabel('team/sub · 1',{exact:true}).isChecked());
  await page.screenshot({path:join(output,`batch-workflow-${width}.png`),fullPage:true});report.screenshots.push(`batch-workflow-${width}.png`);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  if(width===320){
   const responsePromise=page.waitForResponse(response=>response.request().method()==='PATCH'&&response.url().includes('/workflows/'+flow.id));
   await page.getByRole('button',{name:'保存',exact:true}).click();const response=await responsePromise;const body=await response.text();assert(response.ok(),body);
   flow=await api(root+'/workflows/'+flow.id);
  }
 }
 assert.deepEqual(JSON.parse(flow.definition).nodes.find(n=>n.id==='repository').params.repositories,repositories);
 pass('Four widths: group selection includes subgroups, de-duplicates, saves and reads back the actual clone node');
 const started=await api(root+'/workflows/'+flow.id+'/executions',{mode:'manual',revision:'draft',version:flow.version,entry_node_id:'start',inputs:{}});
 const result=await wait(async()=>{const row=await api(root+'/workflow-executions/'+started.id);return ['succeeded','failed','cancelled'].includes(row.state)?row:false;},120000);
 assert.equal(result.state,'succeeded',JSON.stringify(result));
 const attempt=(await api(root+'/workflow-executions/'+result.id+'/node-runs')).items.find(n=>n.node_id==='repository');
 const outputData=await api(root+'/workflow-executions/'+result.id+'/node-runs/'+attempt.id+'/output');
 const rows=outputData.ports[0].items.map(row=>row.json);assert.equal(rows.length,2);
 assert.deepEqual(rows.map(row=>row.repository_url),repositories.map(row=>row.url));
 for(const row of rows){assert.equal(row.execution_host.id,host.id);assert.equal(await readFile(join(row.directory,'proof.txt'),'utf8'),'BATCH_CLONE_CONTENT\n');}
 pass('Real Agent clones both repositories through the saved connection into distinct directories; each result exposes its repository and directory');
 const pipelineDefinition='name: Batch checkout\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - id: checkout_1\n        uses: actions/checkout@v4\n      - id: checkout_2\n        uses: actions/checkout@v4\n      - shell: sh\n        run: |\n          test -f sources/app/proof.txt\n          test -f sources/lib/proof.txt\n          echo PIPELINE_BATCH_OK\n';
 const pipeline=await api(root+'/pipelines',{name:'Two repository checkout build',definition_format:'github-actions',definition:pipelineDefinition,executor_bindings:[{job_id:'build',target_kind:'host',target_ref:host.id,image:'host'}],checkout_bindings:repositories.map((row,index)=>({job_id:'build',step_id:'checkout_'+(index+1),connection_id:connection.id,repository_url:row.url,ref:'',path:'sources/'+row.name}))});
 const pipelineRun=await api(root+'/pipelines/'+pipeline.id+'/runs',{});
 const pipelineResult=await wait(async()=>{const row=await api(root+'/pipeline-runs/'+pipelineRun.id);return ['succeeded','failed','cancelled'].includes(row.state)?row:false;},180000);
 const pipelineLogs=await api(root+'/pipeline-runs/'+pipelineRun.id+'/logs?page_size=200');
 await writeFile(join(output,'pipeline-result.json'),JSON.stringify({run:pipelineResult,logs:pipelineLogs},null,2));
 assert.equal(pipelineResult.state,'succeeded',JSON.stringify(pipelineResult));assert(pipelineLogs.items.some(row=>row.chunk?.includes('PIPELINE_BATCH_OK')));
 pass('Real Actions worker checks out both connection repositories before the build command and verifies both files');
 const forbidden=structuredClone(definition);forbidden.nodes[1].params.repositories=repositories;
 const other=await api('/workspaces',{name:'Other',slug:'batch-clone-other'});
 await api('/workspaces/'+other.id+'/workflows',{name:'Cross workspace',definition:JSON.stringify(forbidden)},'POST',404);
 pass('Cross-workspace batch connection denied');
 assert.deepEqual(errors,[]);report.result='PASS';report.workflow_execution=result.id;
}catch(error){report.result='FAIL';report.error=String(error.stack||error);throw error;}
finally{
 await writeFile(join(output,'result.json'),JSON.stringify(report,null,2));await browser?.close();
 if(runner?.exitCode===null){const stopped=once(runner,'exit');runner.kill();await stopped;}
 if(server?.exitCode===null){const stopped=once(server,'exit');server.kill();await stopped;}
 await new Promise(r=>upstream.close(r));await log.close();
}
