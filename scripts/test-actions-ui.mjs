/** Real browser canvas -> saved Actions YAML -> remote worker -> persisted logs. */
import assert from 'node:assert/strict';
import { parseDocument } from 'yaml';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';
const work=await mkdtemp(join(tmpdir(),'actions-ui-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`, log=await open(join(work,'server.log'),'w');
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};delete env.FERRY_DATABASE_URL;
const server=spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'),[],{env,stdio:['ignore',log.fd,log.fd],windowsHide:true});
let browser,runner,passed=false;
async function eventually(fn) { for(let i=0;i<300;i++){const result=await fn();if(result)return result;await new Promise(r=>setTimeout(r,100));}throw Error('timed out'); }
try {
 await eventually(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok}catch{return false}});
 browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHROME_PATH?{executablePath:process.env.PLAYWRIGHT_CHROME_PATH}:{})});
 const context=await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' },viewport:{width:1440,height:1000},permissions:['clipboard-read','clipboard-write']});
 const post=async(path,data)=>{const response=await context.request.post(origin+'/api'+path,{headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},data});assert(response.ok(),await response.text());return response.json()};
 await post('/setup',{email:'admin@example.invalid',name:'Admin',password:'temporary-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
 const workspace_actions = (await post('/workspaces',{name:'Actions',slug:'actions'})).id;
 const root=`/workspaces/${workspace_actions}`;
 const host=await post(root+'/nodes',{name:'Local runner',connection:'runner'});
 const token=await post(root+'/runner-tokens',{host_id:host.id});
 runner=spawn(resolve('target/debug/ferry-runner'),['--server',origin,'--token',token.token,'--state-dir',join(work,'runner')],{stdio:'ignore',windowsHide:true});
 await eventually(async()=>((await(await context.request.get(origin+'/api'+root+'/nodes/'+host.id)).json()).state==='online'));
 const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.stack??error.message));
 const yamlEditor=page.locator('.monaco-editor .view-lines');
 const readYaml=async()=>{await yamlEditor.click();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.press('ControlOrMeta+C');return page.evaluate(()=>navigator.clipboard.readText());};
 const setYaml=async value=>{await yamlEditor.click();await page.keyboard.press('ControlOrMeta+A');await page.evaluate(text=>navigator.clipboard.writeText(text),value);await page.keyboard.press('ControlOrMeta+V');};
 // Create classification resources through the same shared forms users use.
 await page.goto(origin+`/workspaces/${workspace_actions}/pipelines`);
 await page.getByRole('button',{name:'新建分组'}).click();
 await page.waitForURL('**/pipeline-groups/new');
 assert.equal(await page.getByRole('dialog').count(),0);
 await page.getByLabel('名称',{exact:true}).fill('Browser group');
 await page.getByRole('button',{name:'创建',exact:true}).click();
 await page.waitForURL('**/pipelines');
 await page.goto(origin+`/workspaces/${workspace_actions}/pipelines/new`);
 await page.getByLabel('流水线名称',{exact:true}).fill('Canvas pipeline');
 await page.getByRole('button',{name:'流水线设置'}).click();
 await page.getByRole('combobox',{name:'分组',exact:true}).click();
 await page.getByRole('option',{name:'Browser group',exact:true}).click();
 await page.getByRole('button',{name:'新建标签',exact:true}).click();
 const tagDialog=page.getByRole('dialog');await tagDialog.getByRole('textbox',{name:'名称',exact:true}).fill('Browser tag');await tagDialog.getByRole('button',{name:'确认新建标签',exact:true}).click();
 await tagDialog.waitFor({state:'hidden'});assert(await page.getByRole('checkbox',{name:'Browser tag',exact:true}).isChecked());
 assert.equal(await page.getByRole('button',{name:'新建执行环境',exact:true}).count(),0);
 assert.equal(await page.getByRole('combobox',{name:/目标主机/}).count(),0,'host selection belongs to task panel');
 assert.equal(await page.getByLabel('构建代码来源',{exact:true}).count(),0,'repository selection belongs to checkout node');
 for(const width of [1280,900,390]) {
  await page.setViewportSize({width,height:1000});
  await eventually(async()=>page.locator('form').first().evaluate(form=>{const parent=form.parentElement;const style=getComputedStyle(parent);const available=parent.clientWidth-parseFloat(style.paddingLeft)-parseFloat(style.paddingRight);return form.getBoundingClientRect().width>=available-2;}));
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`pipeline form overflows at ${width}`);
 }
 await page.setViewportSize({width:1440,height:1000});
 await page.getByRole('tab',{name:'高级 YAML',exact:true}).click();
 await setYaml(`# keep workflow comment
name: Browser workflow
on: workflow_dispatch
jobs:
  build:
    name: Compile
    runs-on:
      group: preview-workers # preserve group comment
      labels: [self-hosted]
    env:
      JOB_MARKER: before # preserve job env comment
    steps:
      - name: Compile source
        run: echo BEFORE
`);
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 await page.getByRole('button',{name:/Compile source/}).click();
 await page.locator('summary').filter({hasText:'高级 YAML'}).evaluate(el=>{el.parentElement.open=true});
 await page.getByRole('button',{name:'步骤 YAML',exact:true}).click();
 await setYaml('name: Compile source\nrun: echo ACTIONS_BROWSER_OK ${{ env.WORKFLOW_MARKER }} ${{ env.JOB_MARKER }}\ntimeout-minutes: 2\n');
 await page.getByRole('dialog').getByRole('button',{name:'应用到工作流',exact:true}).click();
 await page.getByRole('button',{name:'关闭配置面板',exact:true}).click();
 await page.getByRole('button',{name:'工作流设置',exact:true}).click();
 await page.getByLabel('工作流名称',{exact:true}).fill('Shared form and YAML');
 await page.getByRole('region',{name:'环境变量',exact:true}).getByRole('button',{name:'添加参数',exact:true}).click();
 await page.getByLabel('环境变量 · 参数名 key',{exact:true}).fill('WORKFLOW_MARKER');
 await page.getByLabel('环境变量.key',{exact:true}).click();
 await page.getByLabel('环境变量.WORKFLOW_MARKER',{exact:true}).fill('WORKFLOW_ENV_OK');
 await page.getByLabel('命令默认设置 · working-directory',{exact:true}).fill('.');
 await page.getByRole('tab',{name:'高级 YAML',exact:true}).click();
 const workflowYaml=await readYaml();
 assert.equal(parseDocument(workflowYaml).toJS().env.WORKFLOW_MARKER,'WORKFLOW_ENV_OK');
 assert.equal(parseDocument(workflowYaml).toJS().defaults.run['working-directory'],'.');
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 await page.getByRole('button',{name:'Compile',exact:true}).click();
 await page.getByLabel('任务名称',{exact:true}).fill('Compile edited');
 await page.getByLabel('环境变量.JOB_MARKER',{exact:true}).fill('JOB_ENV_OK');
 await page.getByLabel('运行条件（if）',{exact:true}).selectOption('success()');
 await page.getByLabel('执行器标签（runs-on） 1',{exact:true}).fill('self-hosted');
 await page.getByRole('button',{name:'添加执行器标签',exact:true}).click();
 await page.getByLabel('执行器标签（runs-on） 2',{exact:true}).fill('linux');
 await page.getByLabel('任务执行器',{exact:true}).selectOption(host.id);
 const grip=page.locator('.react-flow__node[data-id="build"] .job-drag svg');
 await grip.scrollIntoViewIfNeeded();
 const box=await grip.boundingBox();assert(box);
 await page.mouse.move(box.x+box.width/2,box.y+box.height/2);await page.mouse.down();
 await page.mouse.move(box.x+box.width/2+90,box.y+box.height/2+60,{steps:10});await page.mouse.up();

 const canvasNode=page.locator('.react-flow__node[data-id="build"]');
 await canvasNode.focus();
 if(!(await canvasNode.getAttribute('class')).split(' ').includes('selected'))await page.keyboard.press('Enter');
 const nodeX=()=>canvasNode.evaluate(node=>Number(node.style.transform.match(/translate\(([-\d.]+)px/)[1]));
 const beforeKeyboard=await nodeX();
 await canvasNode.press('ArrowRight');
 await eventually(async()=>(await nodeX())>beforeKeyboard);
 const keyboardX=await canvasNode.evaluate(node=>Number(node.style.transform.match(/translate\(([-\d.]+)px/)[1]));
 await page.locator('button[title="在此任务后添加串行任务"]').first().click();await page.getByLabel('任务名称',{exact:true}).fill('Serial successor');await page.getByLabel('任务执行器',{exact:true}).selectOption(host.id);
 await page.locator('button[title="添加并行任务"]').last().click();await page.getByLabel('任务名称',{exact:true}).fill('Parallel sibling');await page.getByLabel('任务执行器',{exact:true}).selectOption(host.id);
 await eventually(async()=>await page.locator('.react-flow__edge').count()===2);
 await page.getByRole('checkbox',{name:'Compile edited',exact:true}).uncheck();await eventually(async()=>await page.locator('.react-flow__edge').count()===1);
 await page.getByRole('checkbox',{name:'Compile edited',exact:true}).check();await eventually(async()=>await page.locator('.react-flow__edge').count()===2);
 await page.getByRole('button',{name:'Compile edited',exact:true}).click();await page.getByRole('checkbox',{name:'Serial successor',exact:true}).click();
 await page.getByRole('alert').filter({hasText:'循环'}).waitFor();assert(!(await page.getByRole('checkbox',{name:'Serial successor',exact:true}).isChecked()));
 await page.getByLabel('任务名称',{exact:true}).fill('Compile intermediate');await page.getByLabel('任务名称',{exact:true}).fill('Compile edited');
 await page.getByRole('tab',{name:'高级 YAML',exact:true}).click();
 const yaml=await readYaml();
 const jobs=parseDocument(yaml).toJS().jobs;assert.deepEqual(jobs.job_1.needs,['build']);assert.deepEqual(jobs.job_2.needs,['build']);
 assert.equal(jobs.build.env.JOB_MARKER,'JOB_ENV_OK');assert.equal(jobs.build.if,'success()');
 assert.deepEqual(jobs.build['runs-on'],{group:'preview-workers',labels:['self-hosted','linux']});
 for(const comment of ['# preserve group comment','# preserve job env comment'])assert(yaml.includes(comment));
 for (const preserved of ['# keep workflow comment', 'ACTIONS_BROWSER_OK', 'timeout-minutes: 2']) assert(yaml.includes(preserved), `YAML lost ${preserved}: ${yaml}`);
 await setYaml('jobs:\n  a:\n    needs: b\n  b:\n    needs: a\n');
 await page.getByRole('alert').filter({hasText:'循环'}).waitFor();
 assert(await page.getByRole('button',{name:'保存',exact:true}).isDisabled());
 await setYaml(yaml);
 await eventually(async()=>await page.getByRole('button',{name:'保存',exact:true}).isEnabled());

 await page.getByRole('button',{name:'保存',exact:true}).click();
 await page.waitForURL(`**/workspaces/${workspace_actions}/pipelines`);
 const list=await(await context.request.get(origin+'/api'+root+'/pipelines')).json();const pipeline=list.items.find(row=>row.name==='Canvas pipeline');assert(pipeline);assert(pipeline.graph_layout.build && pipeline.graph_layout.build.x!==24, "dragged layout was not saved");assert(Math.abs(pipeline.graph_layout.build.x-keyboardX)<0.001,"keyboard layout was not saved (CSS serializes coordinates to three decimals)");
 await page.goto(origin+`/workspaces/${workspace_actions}/pipelines/`+pipeline.id+'/edit');
 await page.getByRole('button',{name:'Compile edited',exact:true}).waitFor();
 await page.getByRole('button',{name:/Compile source/}).click();
 assert.equal(await page.getByLabel('执行命令',{exact:true}).inputValue(),'echo ACTIONS_BROWSER_OK ${{ env.WORKFLOW_MARKER }} ${{ env.JOB_MARKER }}');
 assert.equal(pipeline.environments.length,0);
 assert.equal(pipeline.group_name,'Browser group');assert.equal(pipeline.tag_ids.length,1);
 assert.deepEqual(pipeline.executor_bindings.find(binding=>binding.job_id==='build'),{job_id:'build',target_kind:'host',target_ref:host.id,image:'host'});
 assert.deepEqual(parseDocument(pipeline.definition).toJS().jobs.job_1.needs,['build']);assert.deepEqual(parseDocument(pipeline.definition).toJS().jobs.job_2.needs,['build']);
 await page.getByRole('button',{name:'工作流设置',exact:true}).click();
 assert.equal(await page.getByLabel('工作流名称',{exact:true}).inputValue(),'Shared form and YAML');
 assert.equal(await page.getByLabel('环境变量.WORKFLOW_MARKER',{exact:true}).inputValue(),'WORKFLOW_ENV_OK');
 await page.getByRole('button',{name:'关闭配置面板',exact:true}).click();
 await page.getByRole('button',{name:'Compile edited',exact:true}).click();assert.equal(await page.getByLabel('任务执行器',{exact:true}).inputValue(),host.id);
 for(const width of [1280,900,390]) {await page.setViewportSize({width,height:1000});await eventually(async()=>page.getByLabel('流水线画布',{exact:true}).evaluate(canvas=>{const bounds=canvas.getBoundingClientRect();return [...canvas.querySelectorAll('.react-flow__node')].every(node=>{const box=node.getBoundingClientRect();return box.left>=bounds.left-1&&box.right<=bounds.right+1;});}));assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`canvas overflow at ${width}`);}
 await page.setViewportSize({width:1440,height:1000});
 await page.goto(origin+`/workspaces/${workspace_actions}/pipelines/`+pipeline.id);
 await page.getByRole('button',{name:'触发',exact:true}).click();
 await page.waitForURL(`**/workspaces/${workspace_actions}/pipeline-runs/*`);
 const run={id:page.url().split('/').at(-1)};

 const result=await eventually(async()=>{const row=await(await context.request.get(origin+'/api'+root+'/pipeline-runs/'+run.id)).json();return ['queued','running','cancelling'].includes(row.state)?null:row});
 const logs=await(await context.request.get(origin+'/api'+root+'/pipeline-runs/'+run.id+'/logs')).json();
 assert.equal(result.state,'succeeded',JSON.stringify(result));
 assert(logs.items.some(row=>row.chunk.includes('ACTIONS_BROWSER_OK')));
 // PowerShell echo writes each argument on its own line; require both actual
 // values in this job's persisted output without assuming a shell's line format.
 for(const marker of ['WORKFLOW_ENV_OK','JOB_ENV_OK']) assert(logs.items.some(row=>row.step_index===0&&row.chunk.includes(marker)),`${marker} did not reach the actual worker`);
 await page.goto(origin+`/workspaces/${workspace_actions}/pipelines/`+pipeline.id);
 await page.getByRole('button',{name:'Compile edited',exact:true}).click();
 assert(await page.getByLabel('任务名称',{exact:true}).isDisabled());
 assert.equal(await page.locator('button[title="在此任务后添加串行任务"],button[title="添加并行任务"]').count(),0);
 await page.getByRole('tab',{name:'高级 YAML',exact:true}).click();
 const readonlyYaml=await readYaml();await setYaml('jobs: {}');assert.equal(await readYaml(),readonlyYaml);
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 const readonlyNode=page.locator('.react-flow__node[data-id="build"]');
 await readonlyNode.focus();
 const readonlyPosition=await readonlyNode.evaluate(node=>node.style.transform);
 await page.keyboard.press('ArrowRight');
 await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 assert.equal(await readonlyNode.evaluate(node=>node.style.transform),readonlyPosition,'readonly node moved');
 // Existing custom editors must retain edit access without new classification grants.
 const restricted=await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' } });
 const signup=await restricted.request.post(origin+'/api/users',{headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},data:{email:'editor@example.invalid',name:'Editor',password:'temporary-editor-password'}});assert(signup.ok(),await signup.text());
 const invitation=await(await context.request.get(origin+'/api'+root+'/members/invitation')).json();
 const joined=await restricted.request.post(origin+'/api/workspace-memberships',{headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},data:{code:invitation.code}});assert(joined.ok(),await joined.text());
 const role=await post(root+'/roles',{name:'Existing pipeline editor',permissions:['pipelines.read','pipelines.update','hosts.read','repositories.read']});
 const members=await(await context.request.get(origin+'/api'+root+'/members')).json();
 const member=members.items.find(row=>row.email==='editor@example.invalid');assert(member);
 const assigned=await context.request.patch(origin+'/api'+root+'/members/'+member.id,{headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},data:{role_id:role.id,version:member.version}});assert(assigned.ok(),await assigned.text());
 // Workspace permissions and object ACL are both required by the workflow base.
 const denied=await restricted.request.get(origin+'/api'+root+'/pipelines/'+pipeline.id);assert.equal(denied.status(),403);
 await post(root+'/pipelines/'+pipeline.id+'/acl',{user_id:member.user_id,role:'all'});
 const editor=await restricted.newPage();editor.on('pageerror',error=>errors.push(error.stack??error.message));
 await editor.goto(origin+`/workspaces/${workspace_actions}/pipelines/`+pipeline.id+'/edit');
 await editor.getByLabel('流水线名称').fill('Restricted edit saved');
 await editor.getByRole('button',{name:'保存',exact:true}).click();
 await editor.waitForURL(`**/workspaces/${workspace_actions}/pipelines`);
 const edited=await(await context.request.get(origin+'/api'+root+'/pipelines/'+pipeline.id)).json();
 assert.equal(edited.name,'Restricted edit saved');assert.deepEqual(edited.environment_ids,pipeline.environment_ids);assert.equal(edited.group_id,pipeline.group_id);assert.deepEqual(edited.tag_ids,pipeline.tag_ids);
 await restricted.close();
 await page.setViewportSize({width:390,height:844});
 assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'mobile layout overflows');
 assert.deepEqual(errors,[]);
 passed=true;
 console.log('PASS full-width canvas; inline tag/environment creation; serial/parallel dependencies and cycle rejection; task executor -> saved YAML -> actual runner logs; restricted edits and responsive node visibility');
} catch(error) { console.error('diagnostics:',work); for(const context of browser?.contexts()??[]) for(const page of context.pages()){console.error(await page.locator('body').innerText());await page.screenshot({path:join(work,'failure.png'),fullPage:true});} throw error; }
finally {
 if(runner&&runner.exitCode===null){runner.kill();await new Promise(r=>runner.once('exit',r))}
 if(browser)await browser.close();
 if(server.exitCode===null){server.kill();await new Promise(r=>server.once('exit',r))}
 await log.close();
 // Keep failure evidence outside the repository; successful fixtures are disposable.
 if(passed)await rm(work,{recursive:true,force:true});
}
