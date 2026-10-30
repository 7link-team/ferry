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
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
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
  const workspace_tenant = (await post(owner, '/workspaces', {name:'Tenant',slug:'tenant'})).id;
  const p = await owner.newPage();
  p.on('pageerror', e => errors.push(e.message));
  p.on('dialog', d => d.accept());
  const root=`/workspaces/${workspace_tenant}`;
  const host=await post(owner,root+'/nodes',{name:'Cron UI runner',connection:'runner'});
  await p.goto(`${origin}/workspaces/${workspace_tenant}/nodes/${host.id}?tab=alerts`);
  await p.getByRole('tab',{name:'报警监控'}).waitFor();
  const [eventsResponse]=await Promise.all([
    p.waitForResponse(response=>response.url().includes('/alert-events?') && response.url().includes(`target_ref=${host.id}`) && response.ok()),
    p.getByRole('tab',{name:'告警事件'}).click(),
  ]);
  assert.equal((await eventsResponse.json()).total,0);
  await p.getByRole('tab',{name:'报警规则'}).click();
  await p.getByRole('button',{name:'新增报警规则'}).click();
  await p.getByLabel('目标主机',{exact:false}).waitFor();
  assert.equal(await p.getByLabel('目标主机',{exact:false}).textContent(), 'Cron UI runner');
  await p.goto(`${origin}/workspaces/${workspace_tenant}/alerts`);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('textbox',{name:/^名称/}).fill('Offline browser');
  await p.getByLabel('目标主机',{exact:false}).click();
  await p.getByRole('option',{name:'Cron UI runner',exact:true}).click();
  await p.getByRole('checkbox',{name:'启用',exact:true}).click();
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'Offline browser'}).getByRole('link',{name:'告警事件',exact:true}).click();
  await p.waitForURL(`**/workspaces/${workspace_tenant}/alert-events*`);
  for(let i=0;;i++) {
    const events=await (await owner.request.get(`${origin}/api${root}/alert-events`)).json();
    if(events.total===1)break;
    assert(i<40,'alert did not appear');await new Promise(r=>setTimeout(r,250));
  }
  await p.getByRole('button',{name:'刷新',exact:true}).click();
  await p.getByRole('cell',{name:'Offline browser',exact:true}).waitFor();
  await p.getByRole('row').filter({hasText:'Offline browser'}).getByRole('link',{name:'查看',exact:true}).click();
  await p.getByText('未配置Webhook',{exact:true}).waitFor();
  await p.getByRole('link',{name:'返回规则',exact:true}).click();
  await p.getByRole('row').filter({hasText:'Offline browser'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByRole('textbox',{name:/^名称/}).fill('Edited alert');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('row').filter({hasText:'Edited alert'}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
  await p.waitForFunction(()=>!document.body.innerText.includes('Edited alert'));
  await p.goto(`${origin}/workspaces/${workspace_tenant}/alert-events`);
  await p.getByRole('cell',{name:'Offline browser',exact:true}).waitFor();
  assert.deepEqual(errors,[]);
  console.log('PASS: alert creation, real offline event, history/detail navigation, edit/delete, preserved history and no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
