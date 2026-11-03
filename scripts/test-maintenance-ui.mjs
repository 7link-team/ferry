/** Real Chromium acceptance for workspace preferences and dictionaries. */
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
let browser;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
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
  const get=async path => (await owner.request.get(`${origin}/api${path}`)).json();
  const fillByKeyboard = async (control, value) => {
    await control.click(); await control.press('ControlOrMeta+A');
    await control.pressSequentially(value); await control.press('Tab');
    assert.equal(await control.inputValue(), value, 'Typed value must persist after blur');
  };
  const environment=(await get(root+'/dictionaries')).items.find(row=>row.key==='environment');
  const environmentBefore=await get(root+'/dictionaries/'+environment.id);
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/dictionaries/${environment.id}/edit`);
  const dictionaryType=p.getByLabel('值类型',{exact:true});
  await dictionaryType.waitFor(); assert(await dictionaryType.isEnabled());
  assert.deepEqual(await dictionaryType.locator('option').evaluateAll(nodes=>nodes.map(node=>node.value)),['tab','selection']);
  for(const next of ['tab','selection']) {
    await dictionaryType.click(); await dictionaryType.press(next==='tab'?'Home':'End'); await dictionaryType.press('Enter');
    assert.equal(await dictionaryType.inputValue(),next);
    await p.getByRole('button',{name:'保存',exact:true}).click();
    await p.waitForURL(`**/workspaces/${workspace_tenant}/dictionaries`);
    const saved=await get(root+'/dictionaries/'+environment.id);
    assert.equal(saved.value_type,next);
    assert.deepEqual(JSON.parse(saved.value_json),JSON.parse(environmentBefore.value_json),'Changing compatible types must preserve every option');
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/dictionaries/${environment.id}/edit`);
    await dictionaryType.waitFor(); assert.equal(await dictionaryType.inputValue(),next);
  }
  console.log('PASS: built-in environment dictionary native type switch in both directions, options preserved, real save and reload');
  const initialHeartbeat=(await get(root+'/settings')).items.find(row=>row.key==='heartbeat_timeout_seconds');
  const heartbeatEdit=`${origin}/zh-CN/workspaces/${workspace_tenant}/settings/${initialHeartbeat.id}/edit`;
  await p.goto(heartbeatEdit);
  const valueInput=p.getByRole('spinbutton',{name:'值',exact:true});
  await valueInput.waitFor();
  assert.equal(await p.locator('form section h2').first().innerText(),'当前值');
  assert(await p.getByRole('textbox',{name:/^标识/}).isDisabled());
  await fillByKeyboard(p.getByRole('textbox',{name:/^名称/}),'Runner timeout custom label');
  await fillByKeyboard(p.getByRole('textbox',{name:/^分类/}),'runner');
  await fillByKeyboard(p.getByRole('textbox',{name:/^说明/}),'Custom heartbeat description');
  assert(await p.getByLabel('值类型',{exact:true}).isDisabled());
  await fillByKeyboard(valueInput,'1');
  await p.getByRole('alert').first().waitFor();
  // The editor reports validity to the shared form in a React effect.
  // Wait for that state to commit before asserting the submit guard.
  await p.waitForFunction(button => button.disabled,
    await p.getByRole('button',{name:'保存',exact:true}).elementHandle());
  assert(await p.getByRole('button',{name:'保存',exact:true}).isDisabled());
  assert.equal((await get(root+'/settings/'+initialHeartbeat.id)).value,initialHeartbeat.value,'Invalid number never saved');
  await fillByKeyboard(valueInput,'120');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.waitForURL(`**/workspaces/${workspace_tenant}/settings`);
  await p.goto(heartbeatEdit);
  await valueInput.waitFor(); assert.equal(await valueInput.inputValue(),'120');
  const savedHeartbeat=await get(root+'/settings/'+initialHeartbeat.id);
  assert.equal(savedHeartbeat.value,120); assert.equal(savedHeartbeat.value_json,'120');
  assert.equal(savedHeartbeat.name,'Runner timeout custom label');
  assert.equal(savedHeartbeat.category,'runner');
  assert.equal(savedHeartbeat.description,'Custom heartbeat description');
  assert.equal(await p.getByRole('textbox',{name:/^名称/}).inputValue(),savedHeartbeat.name);
  assert.equal(await p.getByRole('textbox',{name:/^分类/}).inputValue(),savedHeartbeat.category);
  assert.equal(await p.getByRole('textbox',{name:/^说明/}).inputValue(),savedHeartbeat.description);
  await mkdir('data/maintenance/form-e2e',{recursive:true});
  await p.screenshot({path:'data/maintenance/form-e2e/builtin-setting.png',fullPage:true});
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/settings/new`);
  await fillByKeyboard(p.getByRole('textbox',{name:/^标识/}),'editable-setting');
  await fillByKeyboard(p.getByRole('textbox',{name:/^名称/}),'Editable setting');
  await fillByKeyboard(p.getByRole('textbox',{name:/^分类/}),'custom-category');
  await fillByKeyboard(p.getByLabel('值',{exact:true}),'first value');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL(`**/workspaces/${workspace_tenant}/settings`);
  const customSetting=(await get(root+'/settings')).items.find(row=>row.key==='editable-setting');
  assert(customSetting); assert.equal(customSetting.value_json,'"first value"');
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/settings/${customSetting.id}/edit`);
  await p.getByRole('textbox',{name:/^名称/}).waitFor();
  assert(await p.getByRole('textbox',{name:/^标识/}).isDisabled());
  await fillByKeyboard(p.getByRole('textbox',{name:/^名称/}),'Updated setting');
  await fillByKeyboard(p.getByRole('textbox',{name:/^分类/}),'updated-category');
  await fillByKeyboard(p.getByRole('textbox',{name:/^说明/}),'Updated description');
  await fillByKeyboard(p.getByLabel('值',{exact:true}),'updated value');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.waitForURL(`**/workspaces/${workspace_tenant}/settings`);
  const customSaved=await get(root+'/settings/'+customSetting.id);
  assert.equal(customSaved.name,'Updated setting'); assert.equal(customSaved.category,'updated-category');
  assert.equal(customSaved.description,'Updated description'); assert.equal(customSaved.value_json,'"updated value"');
  console.log('PASS: real backend builtin heartbeat keyboard edit, invalid bounds blocked, save/reopen; custom setting metadata and value create/edit');
  await post(owner,root+'/nodes',{name:'Dictionary host',connection:'runner'});
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/settings`);
  await p.getByRole('row').filter({hasText:'default_page_size'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByRole('spinbutton',{name:'值',exact:true}).fill('37');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('cell',{name:'37',exact:true}).waitFor();
  const request=p.waitForRequest(r=>r.url().includes(`/api/workspaces/${workspace_tenant}/nodes?`) && new URL(r.url()).searchParams.get('page_size')==='37');
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/nodes`); await request;
  assert.equal(new URL(p.url()).searchParams.get('page_size'),null,'default page size is not a stale URL override');
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/dictionaries`);
  await p.getByRole('row').filter({hasText:'host-state'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.locator('input[aria-label$=" 值"]').first().waitFor();
  const keys=await p.locator('input[aria-label$=" 值"]').evaluateAll(inputs=>inputs.map(input=>input.value));
  const unknown=keys.indexOf('unknown');
  assert(unknown>=0,'migrated system dictionary retains unknown key');
  await p.getByLabel(`选项 ${unknown+1} 显示名称`,{exact:true}).fill('等待接入');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('cell',{name:'host-state',exact:true}).waitFor();
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/nodes`);
  await p.getByRole('cell',{name:'等待接入',exact:true}).waitFor();
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/dictionaries`);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByLabel('标识',{exact:false}).fill('custom');
  await p.getByRole('textbox',{name:/^名称/}).fill('UI dictionary');
  await p.getByRole('button',{name:'添加选项',exact:true}).click();
  await p.getByLabel('选项 1 值',{exact:true}).fill('one');
  await p.getByLabel('选项 1 显示名称',{exact:true}).fill('One');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'UI dictionary'}).getByRole('link',{name:'复制',exact:true}).click();
  assert.equal(await p.getByLabel('标识',{exact:false}).inputValue(),'custom-copy');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'custom-copy'}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
  await p.waitForFunction(()=>!document.body.innerText.includes('custom-copy'));
  await p.getByRole('row').filter({hasText:'UI dictionary'}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
  await p.waitForFunction(()=>!document.body.innerText.includes('UI dictionary'));
  const heartbeat=(await get(root+'/settings')).items.find(r=>r.key==='heartbeat_timeout_seconds');
  const updated=await owner.request.patch(`${origin}/api${root}/settings/${heartbeat.id}`,{headers,data:{version:heartbeat.version,value:40}});assert(updated.ok());
  const runner=await post(owner,root+'/nodes',{name:'Silent runner',connection:'runner'});
  const {token}=await post(owner,root+'/runner-tokens',{host_id:runner.id});
  const socket=new WebSocket(origin.replace('http:','ws:')+'/api/runners/connect');
  const started=Date.now();
  const closed=new Promise((resolve,reject)=>{
    const timeout=setTimeout(()=>{socket.close();reject(new Error('configured heartbeat timeout was not applied'));},55000);
    socket.addEventListener('close',()=>{clearTimeout(timeout);resolve();});
    socket.addEventListener('error',()=>{clearTimeout(timeout);reject(new Error('runner websocket failed'));});
  });
  socket.addEventListener('open',()=>socket.send(JSON.stringify({version:1,auth:{type:'enroll',token},hostname:'silent',os:'linux',arch:'aarch64',agent_version:'test'})));
  // Receive probes without replying: the configured 40s deadline must close it.
  await closed;
  assert(Date.now()-started>=38000 && Date.now()-started<55000,'real connection uses configured heartbeat timeout');
  assert.equal((await get(root+'/nodes/'+runner.id)).state,'offline');
  console.log('PASS: silent real runner connection closes after configured 40 seconds');
  assert.deepEqual(errors,[]);
  console.log('PASS: shared settings edit, actual 37-row requests, dictionary labels in host table, dictionary create/copy/delete, no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
