/** Real Chromium acceptance for template management and generated resources. */
import assert from 'node:assert/strict';
import ts from 'typescript';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
// Read the real navigation tree without loading browser-only label rendering in Node.
// This acceptance loop uses resource/params only; translated label getters stay lazy.
const navSource = await readFile(new URL('../src/app/nav.ts', import.meta.url), 'utf8');
let isolatedNav = navSource;
for (const [labelImport, replacement] of [
  ['import { commonText, workflowText } from "../shared/i18n/i18n";', 'const commonText = (key: string) => key; const workflowText = (key: string) => key;'],
]) {
  assert(isolatedNav.includes(labelImport), `navigation label import is explicitly isolated: ${labelImport}`);
  isolatedNav = isolatedNav.replace(labelImport, replacement);
}
const navModule = ts.transpileModule(isolatedNav, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
assert(!/^import\s/m.test(navModule), 'navigation fixture must isolate all runtime imports before loading its data URL');
const { nav } = await import(`data:text/javascript;base64,${Buffer.from(navModule).toString('base64')}`);
assert(!JSON.stringify(nav).includes('insight-reports'), 'retired reports have no navigation entry');
assert(!JSON.stringify(nav).includes('code-reviews'), 'retired reviews have no navigation entry');
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
  for (const resource of ['insight-reports', 'code-reviews']) {
    for (const method of ['get', 'post']) {
      const response = await owner.request[method](`${origin}/api${root}/${resource}`, { headers, ...(method === 'post' ? { data: {} } : {}) });
      assert.equal(response.status(), 404, `retired ${resource} ${method} is unavailable`);
      assert(response.headers()['content-type'].includes('application/json'), 'retired API returns JSON, never the SPA');
    }
  }
  console.log('PASS retired reports and reviews have no navigation or GET/POST API');
  const get=async path => {const r=await owner.request.get(`${origin}/api${path}`); assert(r.ok()); return r.json();};
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/tags`);
  await p.getByRole('button',{name:'导入',exact:true}).click();
  const dialog=p.getByRole('dialog');
  await dialog.getByRole('heading',{name:'导入数据'}).waitFor();
  assert(await dialog.getByRole('button',{name:'预览导入'}).isDisabled());
  const templateDownload=p.waitForEvent('download');
  await dialog.getByRole('button',{name:'下载 XLSX 模版'}).click();
  assert.equal((await templateDownload).suggestedFilename(),'tags-template.xlsx');
  const file=dialog.locator('input[type="file"]');
  await file.setInputFiles({name:'tags.csv',mimeType:'text/csv',buffer:Buffer.from('name,color\r\n"UI imported",#123456\r\n')});
  await dialog.getByRole('button',{name:'预览导入'}).click();
  await dialog.getByText('校验通过，将创建 1 条记录。').waitFor();
  assert.equal((await get(root+'/tags')).total,0);
  await dialog.getByRole('button',{name:'确认创建 1 条记录'}).click();
  await dialog.waitFor({state:'hidden'});
  await p.getByRole('row').filter({hasText:'UI imported'}).waitFor();
  assert.equal((await get(root+'/tags')).total,1);
  console.log('PASS import dialog template download, CSV preview without mutation, confirmation and list refresh');
  await p.getByRole('button',{name:'导入',exact:true}).click();
  await file.setInputFiles({name:'invalid.csv',mimeType:'text/csv',buffer:Buffer.from('name,color\n,#123456\n')});
  await dialog.getByRole('button',{name:'预览导入'}).click();
  await dialog.getByText(/校验失败/).waitFor();
  assert.equal(await dialog.getByRole('button',{name:/确认创建/}).count(),0);
  await dialog.getByRole('button',{name:'重新选择文件'}).click();
  await file.setInputFiles({name:'cancel.csv',mimeType:'text/csv',buffer:Buffer.from('name\nCancelled via keyboard\n')});
  await dialog.getByRole('button',{name:'预览导入'}).click();
  await dialog.getByText('校验通过，将创建 1 条记录。').waitFor();
  await p.keyboard.press('Escape');
  await dialog.waitFor({state:'hidden'});
  assert.equal((await get(root+'/tags')).total,1);
  assert.equal(await p.evaluate(()=>document.activeElement.textContent.trim()),'导入');
  console.log('PASS invalid rows prevent confirmation; Escape cancels pending import and restores focus');
  await p.getByRole('button',{name:'导出',exact:true}).click();
  const exported=p.waitForEvent('download');
  await p.getByRole('menuitem',{name:'导出 CSV（当前筛选条件下的全部记录）'}).click();
  const csv=await exported;
  assert.equal(csv.suggestedFilename(),'tags.csv');
  const exportedText=(await readFile(await csv.path(),'utf8')).replace(/^\uFEFF/,'');
  assert.equal(exportedText.split(/\r?\n/)[0],'名称,颜色');
  assert(exportedText.includes('UI imported'));
  const columnsButton=p.getByRole('button').filter({has:p.locator('svg.lucide-columns3')});
  assert.equal(await columnsButton.count(),1);
  await columnsButton.click();
  await p.getByRole('menuitemcheckbox',{name:'颜色',exact:true}).click();
  await p.keyboard.press('Escape');
  await p.getByRole('button',{name:'导出',exact:true}).click();
  const selectedDownload=p.waitForEvent('download');
  await p.getByRole('menuitem',{name:'导出 CSV（当前筛选条件下的全部记录）'}).click();
  const selectedText=(await readFile(await (await selectedDownload).path(),'utf8')).replace(/^\uFEFF/,'');
  assert.equal(selectedText.split(/\r?\n/)[0],'名称');
  assert(selectedText.includes('UI imported'));
  assert(!selectedText.includes('#123456'));
  console.log('PASS real export uses localized visible headers and honors column visibility');
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/credentials`);
  await p.getByRole('button',{name:'导入',exact:true}).click();
  await file.setInputFiles({name:'secret.csv',mimeType:'text/csv',buffer:Buffer.from('name,kind,secret\nPrivate token,token,never-visible-secret\n')});
  await dialog.getByRole('button',{name:'预览导入'}).click();
  await dialog.getByText('校验通过，将创建 1 条记录。').waitFor();
  assert(!(await dialog.innerText()).includes('never-visible-secret'));
  await dialog.getByRole('button',{name:'确认创建 1 条记录'}).click();
  await dialog.waitFor({state:'hidden'});
  assert.equal((await get(root+'/credentials')).total,1);
  const hostBody=await post(owner,root+'/nodes',{name:'Tagged in browser',connection:'runner'});
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/nodes`);
  await p.getByRole('row').filter({hasText:'Tagged in browser'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByRole('checkbox',{name:'UI imported',exact:true}).check();
  const updatingHost=p.waitForResponse(r=>r.url().endsWith('/nodes/'+hostBody.id)&&r.request().method()==='PATCH');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  const updatedHost=await updatingHost; assert(updatedHost.ok(),await updatedHost.text());
  assert.equal((await get(root+'/nodes/'+hostBody.id)).tags[0]?.name,'UI imported');
  await p.getByRole('row').filter({hasText:'Tagged in browser'}).getByText('UI imported',{exact:true}).waitFor();
  await post(owner,root+'/nodes',{name:'Not tagged',connection:'runner'});
  await p.getByRole('combobox',{name:'标签',exact:true}).click();
  await p.getByRole('option',{name:'UI imported',exact:true}).click();
  await p.getByRole('button',{name:'搜索',exact:true}).click();
  await p.getByRole('row').filter({hasText:'Tagged in browser'}).waitFor();
  assert.equal(await p.getByRole('row').filter({hasText:'Not tagged'}).count(),0);
  await p.getByRole('row').filter({hasText:'Tagged in browser'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByRole('checkbox',{name:'UI imported',exact:true}).uncheck();
  await p.getByRole('button',{name:'保存',exact:true}).click();
  const tagged=(await get(root+'/nodes?q=Tagged in browser')).items[0];
  assert.deepEqual(tagged.tag_ids,[]);
  console.log('PASS real host tag multiselect, list labels, filter and removal');
  let pagesChecked=0;
  for (const group of nav) for (const item of group.items) {
    const resource = ({hosts:'nodes', 'host-groups':'node-groups'})[item.resource] || item.resource;
    const loaded=p.waitForResponse(r=>new URL(r.url()).pathname===`/api${root}/${resource}` && r.request().method()==='GET');
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/${resource}${item.params ? '?'+new URLSearchParams(item.params) : ''}`);
    const response=await loaded;
    assert(response.ok(),`${item.resource}: ${await response.text()}`);
    await p.locator('main').waitFor();
    assert(!/还未实现|此页面不支持当前操作/.test(await p.locator('main').innerText()),item.resource);
    pagesChecked++;
  }
  assert(pagesChecked>=25);
  console.log(`PASS ${pagesChecked} navigation and internal pages use real APIs without placeholders`);
  assert.deepEqual(errors,[]);
  console.log('PASS sensitive import preview is masked and no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
