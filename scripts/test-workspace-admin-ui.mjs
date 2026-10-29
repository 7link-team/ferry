/** Real Chromium acceptance for first-run initialization and platform management. */
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
let browser;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(`${origin}/api/status`)).ok) break; } catch {}
    assert(i < 200, 'server startup timed out');
    await new Promise(r => setTimeout(r, 100));
  }
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const errors = [], requests = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('request', r => requests.push(r.url()));

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
  await p.goto(`${origin}/tenant/ops/members`);
  await p.getByRole('cell',{name:'owner@example.invalid',exact:true}).waitFor();
  assert.equal(await p.getByRole('link',{name:'创建',exact:true}).count(),0);
  await p.getByRole('button',{name:'查看邀请码',exact:true}).click();
  const codeInput = p.getByLabel('邀请码',{exact:true});
  await codeInput.waitFor();
  const oldCode = await codeInput.inputValue();
  await p.getByRole('button',{name:'重新生成邀请码',exact:true}).click();
  await p.getByText('已生成新邀请码，旧邀请码已失效。').waitFor();
  const code = await codeInput.inputValue(); assert.notEqual(code,oldCode);
  const guest = await browser.newContext();
  await post(guest,'/users',{email:'guest@example.invalid',name:'Guest',password:'temporary-guest-password'});
  await post(guest,'/workspaces',{name:'Guest home',slug:'guest-home'});
  const g = await guest.newPage();
  await g.goto(`${origin}/guest-home/dashboard/overview`);
  await g.getByRole('button',{name:'Guest home · Guest',exact:true}).click();
  await g.getByRole('menuitem',{name:'创建/加入工作空间',exact:true}).click();
  await g.getByPlaceholder('邀请码',{exact:true}).fill(code);
  await g.getByRole('button',{name:'加入',exact:true}).click();
  await g.waitForURL('**/tenant/**');
  await g.getByRole('heading',{name:'仪表盘',exact:true}).waitFor();
  await p.goto(`${origin}/tenant/ops/roles`);
  const builtin = p.getByRole('row').filter({hasText:'只读'});
  await builtin.getByRole('link',{name:'复制',exact:true}).click();
  await p.getByLabel('名称',{exact:false}).fill('Custom role');
  const hostGroup = p.locator('fieldset p').filter({hasText:/^主机$/}).locator('..');
  await hostGroup.getByRole('checkbox',{name:'新增',exact:true}).check();
  assert.equal(await hostGroup.getByRole('checkbox',{name:'查看',exact:true}).isChecked(),true);
  await hostGroup.getByRole('checkbox',{name:'查看',exact:true}).uncheck();
  assert.equal(await hostGroup.getByRole('checkbox',{name:'新增',exact:true}).isChecked(),false);
  await hostGroup.getByRole('checkbox',{name:'新增',exact:true}).check();
  assert.equal(await hostGroup.getByRole('checkbox',{name:'查看',exact:true}).isChecked(),true);

  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('cell',{name:'Custom role',exact:true}).waitFor();
  const custom = p.getByRole('row').filter({hasText:'Custom role'});
  await custom.getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByLabel('名称',{exact:false}).fill('Custom edited');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('cell',{name:'Custom edited',exact:true}).waitFor();
  await p.goto(`${origin}/tenant/ops/members`);
  const guestRow = p.getByRole('row').filter({hasText:'guest@example.invalid'});
  await guestRow.getByRole('link',{name:'调整角色',exact:true}).click();
  await p.getByRole('combobox').click();
  await p.getByRole('option',{name:'Custom edited',exact:true}).click();
  await p.getByRole('button',{name:'保存',exact:true}).click();
  await p.getByRole('row').filter({hasText:'guest@example.invalid'}).getByRole('cell',{name:'Custom edited',exact:true}).waitFor();
  await p.screenshot({path:'/tmp/workspace-members.png',fullPage:true});
  await p.getByRole('row').filter({hasText:'guest@example.invalid'}).getByRole('button',{name:'移除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认移除',exact:true}).click();
  await p.getByRole('cell',{name:'owner@example.invalid',exact:true}).waitFor();
  await p.waitForFunction(() => !document.body.innerText.includes('guest@example.invalid'));
  await p.goto(`${origin}/tenant/ops/roles`);
  await p.getByRole('row').filter({hasText:'Custom edited'}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
  await p.waitForFunction(() => !document.body.innerText.includes('Custom edited'));
  await p.goto(`${origin}/tenant/dashboard/overview`);
  await p.getByRole('heading',{name:'近期操作',exact:true}).waitFor();
  await p.getByRole('button',{name:'刷新',exact:true}).click();
  await p.getByRole('button',{name:'刷新',exact:true}).waitFor();
  await p.screenshot({path:'/tmp/workspace-dashboard.png',fullPage:true});
  assert.deepEqual(errors,[]);
  console.log('PASS: invitation rotation, existing-user join, role copy/create/edit, member assignment/removal, role deletion, real dashboard, no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
