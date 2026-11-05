/** Real public repositories, actual Agent, Actions dependencies and node execution. */
import { chromium } from 'playwright';
import { spawn } from 'node:child_process';
import { mkdtemp,readFile,writeFile,open,mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join,resolve } from 'node:path';
import { parse, stringify } from 'yaml';
import assert from 'node:assert/strict';

const origin=process.env.FERRY_REAL_ORIGIN || 'http://127.0.0.1:4292';
const work=await mkdtemp(join(tmpdir(),'ferry-real-workflow-'));
const access=JSON.parse(await readFile(process.env.FERRY_REAL_ACCESS || 'data/workflow-preview-20261104/access.json','utf8'));
const resume=process.env.FERRY_REAL_RESUME ? JSON.parse(await readFile(process.env.FERRY_REAL_RESUME,'utf8')) : undefined;
const browser=await chromium.launch({channel:'chrome',headless:true});
let runner;
const report={origin,work,checks:[],projects:{upstream:'https://github.com/golang/text/tree/v0.28.0',downstream:'https://github.com/golang/net/tree/v0.43.0'}};
try {
 const context=await browser.newContext({viewport:{width:1440,height:1000}});
 async function api(path,data,method=data?'POST':'GET',status){
   const response=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});
   assert(status ? response.status()===status : response.ok(),`${method} ${path} ${response.status()} ${await response.text()}`);
   return response.status()===204 ? undefined : response.json();
 }
 await api('/sessions',{email:access.email,password:access.password});
 const workspace=resume?.workspace ? {id:resume.workspace} : await api('/workspaces',{name:'真实构建与节点验收 '+new Date().toISOString(),slug:'real-'+Date.now()});
 const root='/workspaces/'+workspace.id;report.workspace=workspace.id;
 const host=resume?.host ? {id:resume.host} : await api(root+'/nodes',{name:'本机 Agent · 真实开源项目验收',connection:'runner'});
 if (!resume || (await api(root+'/nodes/'+host.id)).state !== 'online') {
 const token=await api(root+'/runner-tokens',{host_id:host.id});report.host=host.id;
 const log=await open(join(work,'agent.log'),'w');
 runner=spawn(resolve('target/debug/ferry-runner'+(process.platform==='win32'?'.exe':'')),['--server',origin,'--token',token.token,'--state-dir',join(work,'agent')],{stdio:['ignore',log.fd,log.fd],windowsHide:true});
 const wait=async(fn,seconds=180)=>{const until=Date.now()+seconds*1000;while(Date.now()<until){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,250));}throw Error('Timed out');};
 await wait(async()=>{assert.equal(runner.exitCode,null,'agent exited');return (await api(root+'/nodes/'+host.id)).state==='online';});
 }
 const wait=async(fn,seconds=180)=>{const until=Date.now()+seconds*1000;while(Date.now()<until){const value=await fn();if(value)return value;await new Promise(r=>setTimeout(r,250));}throw Error('Timed out');};
 report.host=host.id;
 report.checks.push('actual Windows Agent registered and online');
 const workflow=parse(await readFile(new URL("../crates/server/src/templates/examples/dependency-build.yml",import.meta.url),"utf8"));
 await writeFile(join(work,'dependency-build.yml'),stringify(workflow));
 const pipeline=resume?.pipeline ? {id:resume.pipeline} : await api(root+'/pipelines',{name:'真实构建：x/text → x/net/idna',definition_format:'github-actions',definition:stringify(workflow),executor_bindings:['text','idna'].map(job_id=>({job_id,target_kind:'host',target_ref:host.id,image:'host'}))});
 report.pipeline=pipeline.id;
 const started=resume?.pipelineRun ? {id:resume.pipelineRun} : await api(root+'/pipelines/'+pipeline.id+'/runs',{});report.pipelineRun=started.id;
 const result=await wait(async()=>{const r=await api(root+'/pipeline-runs/'+started.id);return ['succeeded','failed','cancelled'].includes(r.state)?r:false;},1200);
 await writeFile(join(work,'pipeline-result.json'),JSON.stringify(result,null,2));
 const logs=await api(root+'/pipeline-runs/'+started.id+'/logs?page_size=200');
 await writeFile(join(work,'pipeline-logs.json'),JSON.stringify(logs,null,2));
 assert.equal(result.state,'succeeded',JSON.stringify(result));
 for(const marker of ['UPSTREAM_TEXT_OK','ARTIFACT_SHA_OK','DOWNSTREAM_IDNA_OK'])assert(logs.items.some(row=>row.chunk?.includes(marker)),marker);
 report.checks.push('two fixed-version public repositories build/test on actual Agent; needs output and uploaded artifact verified by SHA in downstream');
 const directory=join(work,'node-git-text').replaceAll('\\','/');
 const catalog=await api(root+'/workflow-node-catalog');
 const node=(id,type,params,index,settings)=>({id,name:id,type,type_version:1,params,position:[index*320,160],...(settings?{settings}:{})});
 const nodes=[node('start','core.trigger.manual',{input_schema:{type:'object'}},0),
   node('clone','integration.git.repository.clone',{url:'https://github.com/golang/text.git',directory,depth:1},1),
   node('commits','integration.git.repository.log',{directory,limit:5},2),
   node('report','file.encode',{...JSON.parse(await readFile('src/shared/workflow-contracts/workflow-file-nodes.json','utf8')).nodes.find(n=>n.type==='file.encode').example_params,format:'text',output_name:'latest-commits.txt',output_field:'report',value_path:'/stdout'},3,{runtime:'server'})];
 const graph={engine:'ferry-graph',schema:1,nodes,connections:nodes.slice(1).map((n,i)=>({from:{node:nodes[i].id,port:'main',index:0},to:{node:n.id,port:'main',index:0}})),settings:{execution:{runtime:'host',target_kind:'host',target_ref:host.id}}};
 const row=await api(root+'/workflows',{name:'真实节点：Git 拉取 → 提交日志 → 文本报告',definition:JSON.stringify(graph)});report.workflow=row.id;
 const flow=await api(root+'/workflows/'+row.id+'/executions',{mode:'manual',revision:'draft',version:row.version,entry_node_id:'start',inputs:{}});report.execution=flow.id;
 const flowResult=await wait(async()=>{const r=await api(root+'/workflow-executions/'+flow.id);return ['succeeded','failed','cancelled'].includes(r.state)?r:false;},240);
 await writeFile(join(work,'workflow-result.json'),JSON.stringify(flowResult,null,2));assert.equal(flowResult.state,'succeeded',JSON.stringify(flowResult));
 const attempts=await api(root+'/workflow-executions/'+flow.id+'/node-runs');
 const commitAttempt=attempts.items.find(n=>n.node_id==='commits');
 const commits=await api(root+'/workflow-executions/'+flow.id+'/node-runs/'+commitAttempt.id+'/output');
 await writeFile(join(work,'commit-output.json'),JSON.stringify(commits,null,2));
 assert.equal(commits.ports[0].items[0].json.execution_host.id,host.id);assert.match(commits.ports[0].items[0].json.stdout,/^[0-9a-f]{40}\t/);
 report.checks.push('workflow default host dispatches real Git clone/log on Agent and server file node creates report');
 const httpGraph=structuredClone(graph);httpGraph.nodes=[nodes[0],node('github','integration.github.repository.get',{credential_id:'0',owner:'golang',repo:'text'},1)];httpGraph.connections=[{from:{node:'start',port:'main',index:0},to:{node:'github',port:'main',index:0}}];
 const httpRow=await api(root+'/workflows',{name:'真实节点：Agent 访问 GitHub API',definition:JSON.stringify(httpGraph)});report.httpWorkflow=httpRow.id;
 const httpStart=await api(root+'/workflows/'+httpRow.id+'/executions',{mode:'manual',revision:'draft',version:httpRow.version,entry_node_id:'start',inputs:{}});
 const httpResult=await wait(async()=>{const r=await api(root+'/workflow-executions/'+httpStart.id);return ['succeeded','failed','cancelled'].includes(r.state)?r:false;},150);
 assert.equal(httpResult.state,'succeeded',JSON.stringify(httpResult));
 const httpAttempts=await api(root+'/workflow-executions/'+httpStart.id+'/node-runs');
 const httpOutput=await api(root+'/workflow-executions/'+httpStart.id+'/node-runs/'+httpAttempts.items.find(n=>n.node_id==='github').id+'/output');
 assert.equal(httpOutput.ports[0].items[0].json.full_name,'golang/text');assert.equal(httpOutput.ports[0].items[0].json.execution_host.id,host.id);
 report.checks.push('HTTP integration request really originates from Agent and persists host identity');
 const page=await context.newPage();await page.goto(origin+'/zh-CN'+root+'/workflows/'+row.id+'/edit');await page.getByRole('button',{name:'默认执行位置',exact:true}).click();assert.equal(await page.getByRole('combobox',{name:'执行位置',exact:true}).inputValue(),'host');await page.screenshot({path:join(work,'workflow-host.png')});
 await page.getByRole('button',{name:'关闭',exact:true}).last().click();
 for(const app of ['Codeup','Git']) assert(catalog.nodes.some(n=>n.ui.application_name===app));
 await page.goto(origin+'/zh-CN/platform/settings/1/edit');await page.getByText('服务器节点',{exact:true}).waitFor();await page.screenshot({path:join(work,'platform-storage-nodes.png')});
 report.checks.push('real embedded browser shows workflow host selection and platform storage/server nodes');
 report.result='PASS';
} catch(error) {report.result='FAIL';report.error=error.name+': '+error.message.split('\n')[0];throw new Error(report.error);}
finally {await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));await mkdir('data/maintenance',{recursive:true});await writeFile('data/maintenance/workflow-host-real-latest.json',JSON.stringify(report,null,2));console.log('Real acceptance evidence:',work);if(runner?.exitCode===null)runner.kill();await browser.close();}
