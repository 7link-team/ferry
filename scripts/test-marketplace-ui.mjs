/** Real Chromium marketplace source connection selection, synchronization, import and deletion. */
import assert from 'node:assert/strict';
import { createServer as createHttpServer } from 'node:http';
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
  let authorizedSyncs=0;
  const fixture=createHttpServer((request,response)=>{
    const path=new URL(request.url,'http://localhost').pathname;
    response.setHeader('Content-Type','application/json');
    const authorized=(request.headers.authorization??'').replace(/^(Bearer|token) /,'')==='market-browser-token';
    if(!authorized){response.writeHead(401);response.end('{}');return;}
    if(path==='/user'){response.end(JSON.stringify({login:'market-user'}));return;}
    if(path==='/user/repos'){response.end(JSON.stringify([{name:'repository',full_name:'owner/repository',clone_url:`http://127.0.0.1:${fixture.address().port}/owner/repository.git`,default_branch:'main'}]));return;}
    authorizedSyncs++;
    response.end(JSON.stringify({encoding:'base64',content:Buffer.from(JSON.stringify({version:1,templates:[{key:'browser',name:'Browser app',content:{apiVersion:'ferry/v1',kind:'Application',metadata:{name:'web'},services:[{name:'web',image:'busybox:1.36'}]},parameters:[]}]})).toString('base64')}));
  });
  await new Promise(r=>fixture.listen(0,'127.0.0.1',r));
  try {
    const fixtureOrigin=`http://127.0.0.1:${fixture.address().port}`;
    const legacy=await post(owner,root+'/repositories',{name:'Browser repository',url:fixtureOrigin+'/owner/repository.git'});
    const credential=await post(owner,root+'/credentials',{name:'Marketplace PAT',kind:'token',username:'market-user',secret:'market-browser-token',description:''});
    const connection=await post(owner,root+'/code-connections',{name:'Marketplace connection',provider:'github',credential_id:credential.id,base_url:fixtureOrigin,api_url:fixtureOrigin,description:''});
    await p.goto(`${origin}/workspaces/${workspace_tenant}/marketplace-sources`);
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.getByRole('textbox',{name:/^名称/}).fill('Legacy browser source');
    await p.getByLabel('仓库来源',{exact:true}).selectOption('legacy');
    await p.getByLabel('外部仓库',{exact:true}).selectOption(legacy.id);
    await p.getByRole('textbox',{name:/^API 接入点/}).fill(fixtureOrigin);
    await p.getByRole('textbox',{name:/^项目路径/}).fill('owner/repository');
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.waitForURL(`**/workspaces/${workspace_tenant}/marketplace-sources`);
    const originalSources=await(await owner.request.get(`${origin}/api${root}/marketplace-sources`)).json();const legacySource=originalSources.items.find(row=>row.name==='Legacy browser source');assert(legacySource);assert.equal(legacySource.repository_id,legacy.id);
    await p.goto(`${origin}/workspaces/${workspace_tenant}/marketplace-sources/${legacySource.id}/edit`);assert.equal(await p.getByLabel('外部仓库',{exact:true}).inputValue(),legacy.id);
    await p.getByRole('button',{name:'保存',exact:true}).click();await p.waitForURL(`**/workspaces/${workspace_tenant}/marketplace-sources`);
    const retained=await(await owner.request.get(`${origin}/api${root}/marketplace-sources/${legacySource.id}`)).json();assert.equal(retained.repository_id,legacy.id);assert.equal(new URL(retained.api_base).href,new URL(fixtureOrigin).href);assert.equal(retained.project,'owner/repository');
    await p.getByRole('button',{name:'创建',exact:true}).click();await p.getByRole('textbox',{name:/^名称/}).fill('Browser source');
    await p.getByLabel('仓库连接',{exact:true}).selectOption(connection.id);await p.getByLabel('连接仓库',{exact:true}).selectOption(fixtureOrigin+'/owner/repository.git');
    await p.getByRole('textbox',{name:/^API 接入点/}).fill(fixtureOrigin);await p.getByRole('textbox',{name:/^项目路径/}).fill('owner/repository');
    await p.getByRole('button',{name:'创建',exact:true}).click();await p.waitForURL(`**/workspaces/${workspace_tenant}/marketplace-sources`);
    const sources=await(await owner.request.get(`${origin}/api${root}/marketplace-sources`)).json();const source=sources.items.find(row=>row.name==='Browser source');assert(source);
    const repository=await(await owner.request.get(`${origin}/api${root}/repositories/${source.repository_id}`)).json();assert.equal(repository.url,fixtureOrigin+'/owner/repository.git');assert.equal(repository.credential_id,credential.id);
    await p.getByRole('row').filter({has:p.getByRole('cell',{name:'Browser source',exact:true})}).getByRole('button',{name:'立即同步',exact:true}).click();
    await p.getByRole('row').filter({has:p.getByRole('cell',{name:'Browser source',exact:true})}).getByRole('link',{name:'目录',exact:true}).click();
    await p.getByRole('row').filter({hasText:'Browser app'}).getByRole('link',{name:'查看',exact:true}).click();
    await p.getByRole('button',{name:'导入应用模板',exact:true}).click();
    await p.waitForURL(`**/workspaces/${workspace_tenant}/app-templates/*`);
    const templates=await (await owner.request.get(`${origin}/api${root}/app-templates`)).json();
    assert.equal(templates.total,1);assert.equal(templates.items[0].name,'Browser app');
    await p.goto(`${origin}/workspaces/${workspace_tenant}/marketplace-sources`);
    await p.getByRole('row').filter({has:p.getByRole('cell',{name:'Browser source',exact:true})}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
    await p.getByRole('cell',{name:'Browser source',exact:true}).waitFor({state:'detached'});
    assert(authorizedSyncs>0,'marketplace sync used the selected connection credential');
    assert.deepEqual(errors,[]);
    console.log('PASS: legacy source create/edit retention, named connection repository source creation, authenticated HTTP sync, catalog detail, independent template import and source deletion');
  } finally {fixture.closeAllConnections();await new Promise(r=>fixture.close(r));}
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
