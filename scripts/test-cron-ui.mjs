/** Real Chromium acceptance for scheduled tasks and real execution. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const work = await mkdtemp(join(tmpdir(), 'platform-ui-'));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser,runner,frontend;
let uiOrigin=origin;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  // Exercise current source against the isolated real backend before an
  // embedded-server rebuild; CI keeps checking the production bundle by default.
  if(process.env.CRON_UI_DEV) {
    const {createServer:createViteServer}=await import('vite');
    frontend=await createViteServer({server:{host:'127.0.0.1',port:0,strictPort:false,open:false,proxy:{'/api':{target:origin,ws:true}}}});
    await frontend.listen();
    uiOrigin=`http://127.0.0.1:${frontend.httpServer.address().port}`;
  }
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, viewport: { width: 1280, height: 800 } });
  const errors = [];

  const headers = { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' };
  const post = async (ctx, path, data) => {
    const response = await ctx.request.post(`${origin}/api${path}`, { headers, data });
    assert(response.ok(), `${path}: ${await response.text()}`); return response.json();
  };
  await post(context, '/setup', {email:'admin@example.invalid',name:'Admin',password:'temporary-admin-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const owner = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' } });
  await post(owner, '/users', {email:'owner@example.invalid',name:'Owner',password:'temporary-owner-password'});
  const workspace_tenant = (await post(owner, '/workspaces', {name:'Tenant',slug:'tenant'})).id;
  const p = await owner.newPage();
  p.on('pageerror', e => errors.push(e.message));
  p.on('dialog', d => d.accept());
  const root=`/workspaces/${workspace_tenant}`;
  const host=await post(owner,root+'/nodes',{name:'Cron UI runner',connection:'runner'});
  const {token}=await post(owner,root+'/runner-tokens',{host_id:host.id});
  runner=spawn(resolve(process.env.RUNNER_BINARY || 'target/debug/ferry-runner'),['--server',origin,'--token',token,'--state-dir',join(work,'runner')],{stdio:['ignore',log.fd,log.fd],windowsHide:true});
  for(let i=0;;i++) {const r=await owner.request.get(`${origin}/api${root}/nodes/${host.id}`);if((await r.json()).state==='online')break;assert(i<200);await new Promise(r=>setTimeout(r,100));}
  await p.goto(`${uiOrigin}/zh-CN/workspaces/${workspace_tenant}/cron-jobs?scope=host`);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('textbox',{name:/^名称/}).fill('UI command');
  await p.getByRole('textbox',{name:'时区',exact:true}).fill('Asia/Singapore');
  await p.getByRole('combobox',{name:'动作',exact:true}).click();await p.getByRole('option',{name:'执行命令',exact:true}).click();
  await p.getByRole('combobox',{name:'目标类型',exact:true}).click();await p.getByRole('option',{name:'主机',exact:true}).click();
  await p.getByRole('combobox',{name:'目标主机',exact:true}).click();await p.getByRole('option',{name:'Cron UI runner',exact:true}).click();
  await p.getByRole('textbox',{name:/^命令/}).fill('echo CRON_BROWSER');
  const cycle=p.getByLabel('执行周期类型',{exact:true});
  const cron=p.getByLabel('Cron 表达式',{exact:true});
  assert.equal(await cycle.inputValue(),'hours');
  assert.equal(await cycle.locator('option[value="custom"]').isEnabled(),true);
  // Use the native control's actual pointer/keyboard event sequence rather than
  // assigning its DOM value; this catches controlled-select reset regressions.
  await cycle.click();await p.keyboard.press('End');await p.keyboard.press('Enter');
  assert.equal(await cycle.inputValue(),'custom');
  assert.equal(await cron.inputValue(),'0 * * * *','entering custom must preserve the current expression');
  await cron.fill('61 9 * * *');
  await p.getByRole('alert').filter({hasText:'无效'}).waitFor();
  assert.equal(await p.getByRole('button',{name:'创建',exact:true}).isDisabled(),true);
  await cron.fill('0 0 * * *');
  assert.equal(await cycle.inputValue(),'custom','a preset-shaped expression must not exit explicit custom mode');
  await cron.fill('15 8 * * MON-FRI');
  assert.equal(await p.getByRole('button',{name:'创建',exact:true}).isEnabled(),true);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'UI command'}).waitFor();
  let saved=(await (await owner.request.get(`${origin}/api${root}/cron-jobs`)).json()).items.find(row=>row.name==='UI command');
  assert.equal(saved.schedule,'15 8 * * MON-FRI','custom expression persists through the real API');
  await p.getByRole('row').filter({hasText:'UI command'}).getByRole('button',{name:'立即执行',exact:true}).click();
  await p.waitForURL(`**/workspaces/${workspace_tenant}/cron-runs/*?scope=host`);
  await p.locator('p.font-mono').filter({hasText:'退出码 0'}).waitFor();
  const id=new URL(p.url()).pathname.split('/').pop();
  const run=await (await owner.request.get(`${origin}/api${root}/cron-runs/${id}`)).json();
  assert.equal(run.state,'succeeded');assert(run.output.includes('CRON_BROWSER'));
  await p.getByRole('link',{name:'返回任务',exact:true}).click();
  await p.getByRole('link',{name:'执行历史',exact:true}).click();
  await p.getByRole('cell',{name:'成功',exact:true}).waitFor();
  await p.goto(`${uiOrigin}/zh-CN/workspaces/${workspace_tenant}/cron-jobs?scope=host`);
  await p.getByRole('row').filter({hasText:'UI command'}).getByRole('link',{name:'编辑',exact:true}).click();
  await cron.waitFor();
  assert.equal(await cycle.inputValue(),'custom');
  assert.equal(await cron.inputValue(),'15 8 * * MON-FRI');
  await cycle.click();await p.keyboard.press('Home');await p.keyboard.press('ArrowDown');await p.keyboard.press('Enter');
  assert.equal(await cycle.inputValue(),'hours');
  await p.getByLabel('间隔小时',{exact:true}).fill('4');
  assert.equal(await cron.inputValue(),'0 */4 * * *');
  await cycle.click();await p.keyboard.press('End');await p.keyboard.press('Enter');
  assert.equal(await cycle.inputValue(),'custom');
  assert.equal(await cron.inputValue(),'0 */4 * * *','returning to custom preserves the selected preset value');
  await p.getByRole('textbox',{name:/^名称/}).fill('UI edited');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('row').filter({hasText:'UI edited'}).waitFor();
  saved=await (await owner.request.get(`${origin}/api${root}/cron-jobs/${saved.id}`)).json();
  assert.equal(saved.schedule,'0 */4 * * *');
  await p.getByRole('row').filter({hasText:'UI edited'}).getByRole('link',{name:'编辑',exact:true}).click();
  await cron.waitFor();
  assert.equal(await cron.inputValue(),'0 */4 * * *');
  assert.equal(await cycle.inputValue(),'hours','reopening infers the preset from its saved expression');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('row').filter({hasText:'UI edited'}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
  await p.waitForFunction(()=>!document.body.innerText.includes('UI edited'));
  for (const scope of ['build','deploy']) {
    await p.goto(`${uiOrigin}/zh-CN/workspaces/${workspace_tenant}/cron-jobs/new?scope=${scope}`);
    assert.equal(await p.getByRole('combobox',{name:'所属模块',exact:true}).count(),0,'module is supplied by its navigation entry');
    assert.equal(await p.getByRole('combobox',{name:'目标类型',exact:true}).count(),0,'action determines the target type');
    await p.getByRole('combobox',{name:'动作',exact:true}).click();
    await p.getByRole('option',{name:scope==='build'?'触发流水线':'重新部署当前配置',exact:true}).click();
    await p.getByRole('combobox',{name:scope==='build'?'目标流水线':'目标应用',exact:true}).waitFor();
    if(scope==='deploy') {
      await p.getByRole('combobox',{name:'动作',exact:true}).click();
      await p.getByRole('option',{name:'同步市场目录',exact:true}).click();
      await p.getByRole('combobox',{name:'市场来源',exact:true}).waitFor();
      assert.equal(await p.getByRole('combobox',{name:'目标应用',exact:true}).count(),0);
    }
  }
  const graph={engine:'ferry-graph',schema:1,settings:{timezone:'UTC'},nodes:[
    {id:'start',name:'定时入口',type:'core.trigger.manual',type_version:1,position:[0,0],params:{input_schema:{type:'object',properties:{message:{type:'string'}},required:['message']}}}
  ],connections:[]};
  const workflow=await post(owner,root+'/workflows',{name:'定时工作流验收',definition:JSON.stringify(graph)});
  await post(owner,root+`/workflows/${workflow.id}/publications`,{version:workflow.version,revision_id:workflow.draft_revision_id,name:'v1'});
  await p.goto(`${uiOrigin}/zh-CN${root}/cron-jobs/new?scope=build`);
  await p.locator('a[href*="/workflows"]').filter({hasText:/^工作流$/}).first().waitFor();
  await p.locator('a[href*="/pipelines"]').filter({hasText:/^流水线$/}).first().waitFor();
  await p.getByRole('textbox',{name:/^名称/}).fill('节点工作流定时任务');
  await p.getByRole('textbox',{name:'时区',exact:true}).fill('UTC');
  await p.getByRole('combobox',{name:'动作',exact:true}).click();
  await p.getByRole('option',{name:'触发工作流',exact:true}).click();
  await p.getByRole('combobox',{name:'工作流',exact:true}).click();
  await p.getByRole('option',{name:'定时工作流验收',exact:true}).click();
  const input=p.getByRole('textbox',{name:/message/});
  await input.waitFor();
  assert.equal(await p.getByRole('button',{name:'创建',exact:true}).isDisabled(),true,'required workflow inputs block save');
  await input.fill('浏览器真实定时输入');
  await p.getByRole('combobox',{name:'动作',exact:true}).click();
  await p.getByRole('option',{name:'触发流水线',exact:true}).click();
  await p.getByRole('combobox',{name:'目标流水线',exact:true}).waitFor();
  assert.equal(await input.count(),0,'changing actions clears the previous entry inputs');
  await p.getByRole('combobox',{name:'动作',exact:true}).click();
  await p.getByRole('option',{name:'触发工作流',exact:true}).click();
  assert(!(await p.getByRole('combobox',{name:'工作流',exact:true}).innerText()).includes('定时工作流验收'),'old target is cleared');
  await p.getByRole('combobox',{name:'工作流',exact:true}).click();
  await p.getByRole('option',{name:'定时工作流验收',exact:true}).click();
  await input.waitFor();assert.equal(await input.inputValue(),'');
  await input.fill('浏览器真实定时输入');
  await mkdir(resolve('data/maintenance/workflow-cron-browser'),{recursive:true});
  for(const width of [1280,900,390,320]) {
    await p.setViewportSize({width,height:850});
    await input.scrollIntoViewIfNeeded();
    assert(await p.evaluate(()=>document.documentElement.scrollWidth<=document.documentElement.clientWidth+1),`no page overflow at ${width}`);
    await p.screenshot({path:resolve(`data/maintenance/workflow-cron-browser/form-${width}.png`),fullPage:true});
  }
  await p.setViewportSize({width:1280,height:850});
  await p.getByRole('button',{name:'创建',exact:true}).click();
  const row=p.getByRole('row').filter({hasText:'节点工作流定时任务'});
  await row.waitFor();
  await row.getByRole('button',{name:'立即执行',exact:true}).click();
  await p.waitForURL(`**${root}/cron-runs/*?scope=build`);
  const workflowRunId=new URL(p.url()).pathname.split('/').pop();
  let workflowRun;
  for(let i=0;;i++) {
    workflowRun=await(await owner.request.get(`${origin}/api${root}/cron-runs/${workflowRunId}`)).json();
    if(!['queued','running'].includes(workflowRun.state))break;
    assert(i<100,'workflow execution timeout');await new Promise(r=>setTimeout(r,200));
  }
  assert.equal(workflowRun.state,'succeeded');
  assert.deepEqual(JSON.parse(workflowRun.config).inputs,{message:'浏览器真实定时输入'});
  assert.equal(JSON.parse(workflowRun.config).scope,'build');
  assert.equal(JSON.parse(workflowRun.config).target_kind,'workflow');
  await p.getByRole('link',{name:'查看下游执行',exact:true}).click();
  await p.waitForURL(`**${root}/workflows/${workflow.id}?execution=${workflowRun.downstream_id}`);
  assert(!new URL(p.url()).pathname.startsWith('/api/'));
  await p.getByRole('status').filter({hasText:/成功/}).first().waitFor();
  await p.getByText('定时入口',{exact:true}).first().waitFor();
  await p.screenshot({path:resolve('data/maintenance/workflow-cron-browser/execution.png'),fullPage:true});
  assert.deepEqual(errors,[]);
  console.log('PASS: published workflow selection, required inputs, four responsive widths, real workflow execution and UI execution deep link');
  console.log('PASS: native custom cron selection/preserved expression/invalid save blocking/preset switching/API save and reopen; shared cron creation/edit/delete, real runner execution with live detail polling, history navigation, three scope defaults, no browser errors');
} finally {
  if (runner && runner.exitCode===null) {runner.kill();await new Promise(r=>runner.once("exit",r));}
  if (browser) await browser.close();
  if (frontend) await frontend.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
