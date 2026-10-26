/** Real Chromium acceptance for scheduled tasks and real execution. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open } from 'node:fs/promises';
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
let browser,runner;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const errors = [];

  const headers = { 'X-Ferry-Request': '1' };
  const post = async (ctx, path, data) => {
    const response = await ctx.request.post(`${origin}/api${path}`, { headers, data });
    assert(response.ok(), `${path}: ${await response.text()}`); return response.json();
  };
  await post(context, '/setup', {email:'admin@example.invalid',name:'Admin',password:'temporary-admin-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const owner = await browser.newContext();
  await post(owner, '/users', {email:'owner@example.invalid',name:'Owner',password:'temporary-owner-password'});
  await post(owner, '/workspaces', {name:'Tenant',slug:'tenant'});
  const p = await owner.newPage();
  p.on('pageerror', e => errors.push(e.message));
  p.on('dialog', d => d.accept());
  const root='/workspaces/tenant';
  const host=await post(owner,root+'/hosts',{name:'Cron UI runner',connection:'runner'});
  const {token}=await post(owner,root+'/runner-tokens',{host_id:host.id});
  runner=spawn(resolve('target/debug/ferry-runner'),['--server',origin,'--token',token,'--state-dir',join(work,'runner')],{stdio:['ignore',log.fd,log.fd],windowsHide:true});
  for(let i=0;;i++) {const r=await owner.request.get(`${origin}/api${root}/hosts/${host.id}`);if((await r.json()).state==='online')break;assert(i<200);await new Promise(r=>setTimeout(r,100));}
  await p.goto(`${origin}/tenant/host/cron-jobs`);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('textbox',{name:/^名称/}).fill('UI command');
  await p.getByLabel('时区',{exact:false}).fill('Asia/Singapore');
  await p.getByLabel('动作',{exact:false}).click();await p.getByRole('option',{name:'执行命令',exact:true}).click();
  await p.getByLabel('目标类型',{exact:false}).click();await p.getByRole('option',{name:'主机',exact:true}).click();
  await p.getByLabel('目标主机',{exact:false}).click();await p.getByRole('option',{name:'Cron UI runner',exact:true}).click();
  await p.getByRole('textbox',{name:/^命令/}).fill('echo CRON_BROWSER');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'UI command'}).getByRole('button',{name:'立即执行',exact:true}).click();
  await p.waitForURL('**/host/cron-runs/*');
  await p.locator('p.font-mono').filter({hasText:'退出码 0'}).waitFor();
  const id=new URL(p.url()).pathname.split('/').pop();
  const run=await (await owner.request.get(`${origin}/api${root}/cron-runs/${id}`)).json();
  assert.equal(run.state,'succeeded');assert(run.output.includes('CRON_BROWSER'));
  await p.getByRole('link',{name:'返回任务',exact:true}).click();
  await p.getByRole('link',{name:'执行历史',exact:true}).click();
  await p.getByRole('cell',{name:'成功',exact:true}).waitFor();
  await p.goto(`${origin}/tenant/host/cron-jobs`);
  await p.getByRole('row').filter({hasText:'UI command'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByRole('textbox',{name:/^名称/}).fill('UI edited');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('row').filter({hasText:'UI edited'}).getByRole('button',{name:'删除',exact:true}).click();
  await p.waitForFunction(()=>!document.body.innerText.includes('UI edited'));
  for (const scope of ['build','deploy']) {
    await p.goto(`${origin}/tenant/${scope}/cron-jobs/new`);
    const selected=await p.getByLabel('所属模块',{exact:false}).innerText();
    assert(selected.includes(scope==='build'?'构建':'部署'));
    await p.getByLabel('动作',{exact:false}).click();
    await p.getByRole('option',{name:scope==='build'?'触发流水线':'重新部署当前配置',exact:true}).waitFor();
    await p.keyboard.press('Escape');
  }
  assert.deepEqual(errors,[]);
  console.log('PASS: shared cron creation/edit/delete, real runner execution with live detail polling, history navigation, three scope defaults, no browser errors');
} finally {
  if (runner && runner.exitCode===null) {runner.kill();await new Promise(r=>runner.once("exit",r));}
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
