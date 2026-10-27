/** Real browser canvas -> saved Actions YAML -> remote worker -> persisted logs. */
import assert from 'node:assert/strict';
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
const server=spawn(resolve('target/debug/ferry-server'),[],{env,stdio:['ignore',log.fd,log.fd],windowsHide:true});
let browser,runner,passed=false;
async function eventually(fn) { for(let i=0;i<300;i++){const result=await fn();if(result)return result;await new Promise(r=>setTimeout(r,100));}throw Error('timed out'); }
try {
 await eventually(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok}catch{return false}});
 browser=await chromium.launch({headless:true});
 const context=await browser.newContext({viewport:{width:1440,height:1000}});
 const post=async(path,data)=>{const response=await context.request.post(origin+'/api'+path,{headers:{'X-Ferry-Request':'1'},data});assert(response.ok(),await response.text());return response.json()};
 await post('/setup',{email:'admin@example.invalid',name:'Admin',password:'temporary-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
 await post('/workspaces',{name:'Actions',slug:'actions'});
 const root='/workspaces/actions';
 const host=await post(root+'/hosts',{name:'Local runner',connection:'runner'});
 const token=await post(root+'/runner-tokens',{host_id:host.id});
 runner=spawn(resolve('target/debug/ferry-runner'),['--server',origin,'--token',token.token,'--state-dir',join(work,'runner')],{stdio:'ignore',windowsHide:true});
 await eventually(async()=>((await(await context.request.get(origin+'/api'+root+'/hosts/'+host.id)).json()).state==='online'));
 const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));
 // Create classification resources through the same shared forms users use.
 await page.goto(origin+'/actions/build/pipeline-groups/new');
 await page.getByLabel(/^名称/).fill('Browser group');
 await page.getByRole('button',{name:'创建',exact:true}).click();
 await page.waitForURL('**/actions/build/pipeline-groups');
 for(const name of ['Browser staging','Browser production']) {
  await page.goto(origin+'/actions/build/pipeline-environments/new');
  await page.getByLabel(/^名称/).fill(name);
  await page.getByRole('combobox',{name:/默认主机/}).click();
  await page.getByRole('option',{name:'Local runner',exact:true}).click();
  await page.getByRole('button',{name:'创建',exact:true}).click();
  await page.waitForURL('**/actions/build/pipeline-environments');
 }
 await page.goto(origin+'/actions/build/pipelines/new');
 await page.getByLabel(/^名称/).fill('Canvas pipeline');
 await page.getByRole('combobox',{name:'分组',exact:true}).click();
 await page.getByRole('option',{name:'Browser group',exact:true}).click();
 await page.getByRole('checkbox',{name:'Browser staging',exact:true}).check();
 await page.getByRole('checkbox',{name:'Browser production',exact:true}).check();

 await page.getByRole('combobox',{name:/目标主机/}).click();
 await page.getByRole('option',{name:'Local runner',exact:true}).click();
 await page.getByRole('tab',{name:'YAML',exact:true}).click();
 await page.getByLabel('GitHub Actions YAML',{exact:true}).fill(`# keep workflow comment
name: Browser workflow
on: workflow_dispatch
jobs:
  build:
    name: Compile
    runs-on: self-hosted
    steps:
      - name: Compile source
        run: echo BEFORE
`);
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 await page.getByRole('button',{name:/Compile source/}).click();
 await page.getByLabel('执行命令',{exact:true}).fill('echo ACTIONS_BROWSER_OK');
 await page.getByLabel('超时（分钟）',{exact:true}).fill('2');
 await page.getByRole('button',{name:'关闭配置面板',exact:true}).click();
 await page.getByRole('button',{name:'Compile',exact:true}).click();
 await page.getByLabel('任务名称',{exact:true}).fill('Compile edited');
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
 await page.getByRole('tab',{name:'YAML',exact:true}).click();
 const yaml=await page.getByLabel('GitHub Actions YAML',{exact:true}).inputValue();
 assert(yaml.includes('# keep workflow comment')&&yaml.includes('ACTIONS_BROWSER_OK')&&yaml.includes('timeout-minutes: 2'));
 await page.getByLabel('GitHub Actions YAML',{exact:true}).fill('jobs:\n  a:\n    needs: b\n  b:\n    needs: a\n');
 assert((await page.getByRole('alert').allTextContents()).some(text=>text.includes('循环')));
 assert(await page.getByRole('button',{name:'创建',exact:true}).isDisabled());
 await page.getByLabel('GitHub Actions YAML',{exact:true}).fill(yaml);
 await eventually(async()=>await page.getByRole('button',{name:'创建',exact:true}).isEnabled());

 await page.getByRole('button',{name:'创建',exact:true}).click();
 await page.waitForURL('**/actions/build/pipelines');
 const list=await(await context.request.get(origin+'/api'+root+'/pipelines')).json();const pipeline=list.items.find(row=>row.name==='Canvas pipeline');assert(pipeline);assert(pipeline.graph_layout.build && pipeline.graph_layout.build.x!==24, "dragged layout was not saved");assert.equal(pipeline.graph_layout.build.x,keyboardX,"keyboard layout was not saved");
 await page.goto(origin+'/actions/build/pipelines/'+pipeline.id+'/edit');
 await page.getByRole('button',{name:'Compile edited',exact:true}).waitFor();
 await page.getByRole('button',{name:/Compile source/}).click();
 assert.equal(await page.getByLabel('执行命令',{exact:true}).inputValue(),'echo ACTIONS_BROWSER_OK');
 assert.equal(pipeline.environments.length,2);
 assert.equal(pipeline.group_name,'Browser group');
 await page.goto(origin+'/actions/build/pipelines/'+pipeline.id);
 await page.getByRole('button',{name:'触发',exact:true}).click();
 const dialog=page.getByRole('dialog');
 assert(await dialog.getByRole('button',{name:'确认触发',exact:true}).isDisabled());
 await dialog.getByRole('button',{name:'取消',exact:true}).click();
 assert.equal((await(await context.request.get(origin+'/api'+root+'/pipeline-runs')).json()).total,0,'cancel unexpectedly triggered run');
 await page.getByRole('button',{name:'触发',exact:true}).click();
 await dialog.getByRole('combobox',{name:/执行环境/}).click();
 await page.getByRole('option',{name:'Browser production',exact:true}).click();
 await dialog.getByRole('button',{name:'确认触发',exact:true}).click();
 await page.waitForURL('**/actions/build/runs/*');
 const run={id:page.url().split('/').at(-1)};

 const result=await eventually(async()=>{const row=await(await context.request.get(origin+'/api'+root+'/pipeline-runs/'+run.id)).json();return ['queued','running','cancelling'].includes(row.state)?null:row});
 assert.equal(result.state,'succeeded',JSON.stringify(result));
 assert.equal(result.environment_name,'Browser production');
 const logs=await(await context.request.get(origin+'/api'+root+'/pipeline-runs/'+run.id+'/logs')).json();assert(logs.items.some(row=>row.chunk.includes('ACTIONS_BROWSER_OK')));
 await page.goto(origin+'/actions/build/pipelines/'+pipeline.id);
 await page.getByRole('button',{name:'Compile edited',exact:true}).click();
 assert(await page.getByLabel('任务名称',{exact:true}).isDisabled());
 assert.equal(await page.getByRole('button',{name:'添加任务',exact:true}).count(),0);
 await page.getByRole('tab',{name:'YAML',exact:true}).click();
 assert(await page.getByLabel('GitHub Actions YAML',{exact:true}).evaluate(node=>node.readOnly));
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 const readonlyNode=page.locator('.react-flow__node[data-id="build"]');
 await readonlyNode.focus();
 const readonlyPosition=await readonlyNode.evaluate(node=>node.style.transform);
 await page.keyboard.press('ArrowRight');
 await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
 assert.equal(await readonlyNode.evaluate(node=>node.style.transform),readonlyPosition,'readonly node moved');
 // Existing custom editors must retain edit access without new classification grants.
 const restricted=await browser.newContext();
 const signup=await restricted.request.post(origin+'/api/users',{headers:{'X-Ferry-Request':'1'},data:{email:'editor@example.invalid',name:'Editor',password:'temporary-editor-password'}});assert(signup.ok(),await signup.text());
 const invitation=await(await context.request.get(origin+'/api'+root+'/members/invitation')).json();
 const joined=await restricted.request.post(origin+'/api/workspaces/actions/join',{headers:{'X-Ferry-Request':'1'},data:{code:invitation.code}});assert(joined.ok(),await joined.text());
 const role=await post(root+'/roles',{name:'Existing pipeline editor',permissions:['pipelines.read','pipelines.update','hosts.read','repositories.read']});
 const members=await(await context.request.get(origin+'/api'+root+'/members')).json();
 const member=members.items.find(row=>row.email==='editor@example.invalid');assert(member);
 const assigned=await context.request.patch(origin+'/api'+root+'/members/'+member.id,{headers:{'X-Ferry-Request':'1'},data:{role_id:role.id,version:member.version}});assert(assigned.ok(),await assigned.text());
 const editor=await restricted.newPage();editor.on('pageerror',error=>errors.push(error.message));
 await editor.goto(origin+'/actions/build/pipelines/'+pipeline.id+'/edit');
 await editor.getByLabel(/^名称/).fill('Restricted edit saved');
 await editor.getByRole('button',{name:'保存',exact:true}).click();
 await editor.waitForURL('**/actions/build/pipelines');
 const edited=await(await context.request.get(origin+'/api'+root+'/pipelines/'+pipeline.id)).json();
 assert.equal(edited.name,'Restricted edit saved');assert.deepEqual(edited.environment_ids,pipeline.environment_ids);assert.equal(edited.group_id,pipeline.group_id);
 await restricted.close();
 await page.setViewportSize({width:390,height:844});
 assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'mobile layout overflows');
 assert.deepEqual(errors,[]);
 passed=true;
 console.log('PASS canvas step/job panel -> comment-preserving YAML -> save/reopen -> actual runner -> remote logs; mobile and browser errors');
} catch(error) { console.error('diagnostics:',work); for(const context of browser?.contexts()??[]) for(const page of context.pages()){console.error(await page.locator('body').innerText());await page.screenshot({path:join(work,'failure.png'),fullPage:true});} throw error; }
finally {
 if(runner&&runner.exitCode===null){runner.kill();await new Promise(r=>runner.once('exit',r))}
 if(browser)await browser.close();
 if(server.exitCode===null){server.kill();await new Promise(r=>server.once('exit',r))}
 await log.close();
 // Keep failure evidence outside the repository; successful fixtures are disposable.
 if(passed)await rm(work,{recursive:true,force:true});
}
