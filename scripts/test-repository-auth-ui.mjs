/** Real Chromium acceptance for scheduled tasks and real execution. */
import assert from 'node:assert/strict';
import { createServer as createHttpServer } from 'node:http';
import { createHash, randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
const codes=new Map();
const provider=createHttpServer(async(request,response)=>{
  const url=new URL(request.url,'http://localhost');
  if(url.pathname==='/login/oauth/authorize') {
    const code=randomUUID();codes.set(code,url.searchParams);
    const callback=new URL(url.searchParams.get('redirect_uri'));
    callback.searchParams.set('code',code);callback.searchParams.set('state',url.searchParams.get('state'));
    response.writeHead(302,{Location:callback.href});response.end();return;
  }
  if(url.pathname==='/login/oauth/access_token') {
    let body='';for await(const part of request)body+=part;
    const form=new URLSearchParams(body),auth=codes.get(form.get('code'));
    assert(auth);codes.delete(form.get('code'));
    assert.equal(createHash('sha256').update(form.get('code_verifier')).digest('base64url'),auth.get('code_challenge'));
    assert.equal(form.get('client_secret'),'fixture-client-secret');
    response.setHeader('Content-Type','application/json');response.end(JSON.stringify({access_token:'fixture-browser-access-token',refresh_token:'fixture-browser-refresh-token',expires_in:3600}));return;
  }
  assert.equal(url.pathname,'/user');assert.equal(request.headers.authorization,'Bearer fixture-browser-access-token');
  response.setHeader('Content-Type','application/json');response.end(JSON.stringify({login:'browser-authorized-user'}));
});
await new Promise(r=>provider.listen(0,'127.0.0.1',r));
const providerOrigin=`http://127.0.0.1:${provider.address().port}`;
const work = await mkdtemp(join(tmpdir(), 'platform-ui-'));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
Object.assign(env,{REPOSITORY_OAUTH_PUBLIC_URL:origin,REPOSITORY_OAUTH_GITHUB_CLIENT_ID:'fixture-client-id',REPOSITORY_OAUTH_GITHUB_CLIENT_SECRET:'fixture-client-secret',REPOSITORY_OAUTH_GITHUB_BASE_URL:providerOrigin,REPOSITORY_OAUTH_GITHUB_API_URL:providerOrigin,RUST_LOG:'ferry_server=debug'});
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
  await p.goto(`${origin}/tenant/build/repositories`);
  assert.equal(await p.locator('nav[aria-label="构建"]').getByRole('link',{name:'平台授权',exact:true}).count(),0);
  assert.equal(await p.locator('nav[aria-label="构建"]').getByRole('link',{name:'API 令牌',exact:true}).count(),0);
  await p.getByRole('navigation',{name:'仓库页面',exact:true}).getByRole('link',{name:'平台授权',exact:true}).click();
  for (const name of ['GitLab','Gitee']) {
    const row=p.getByRole('row').filter({hasText:name});
    assert(await row.getByRole('button',{name:'网页授权',exact:true}).isDisabled());
    assert((await row.innerText()).includes('尚未配置'));
  }
  if (process.env.UI_SCREENSHOT) await p.screenshot({ path: process.env.UI_SCREENSHOT });
  await p.getByRole('row').filter({hasText:'GitHub'}).getByRole('button',{name:'网页授权',exact:true}).click();
  await p.waitForURL('**/build/repository-auth?authorization=connected');
  await p.getByRole('cell',{name:'browser-authorized-user',exact:true}).waitFor();
  const first=await (await owner.request.get(`${origin}/api${root}/credentials`)).json();assert.equal(first.total,1);
  await p.getByRole('row').filter({hasText:'GitHub'}).getByRole('button',{name:'重新授权',exact:true}).click();
  await p.waitForURL('**/build/repository-auth?authorization=connected');
  await p.getByRole('cell',{name:'browser-authorized-user',exact:true}).waitFor();
  const second=await (await owner.request.get(`${origin}/api${root}/credentials`)).json();assert.equal(second.total,1);assert.equal(second.items[0].id,first.items[0].id);
  await p.getByRole('row').filter({hasText:'GitHub'}).getByRole('link',{name:'查看凭据',exact:true}).click();
  await p.waitForURL('**/build/credentials/*');
  assert(!(await p.locator('body').innerText()).includes('fixture-browser-access-token'));
  await p.goto(`${origin}/tenant/build/repository-auth`);
  await p.getByRole('row').filter({hasText:'Codeup'}).getByRole('link',{name:'使用个人访问令牌',exact:true}).click();
  await p.waitForURL('**/build/api-tokens/new');
  assert((await p.locator('main').innerText()).includes('不是调用本平台 API 的令牌'));
  assert((await p.getByLabel('类型',{exact:false}).innerText()).includes('令牌'));
  assert.deepEqual(errors,[]);
  const logs=await readFile(join(work,'server.log'),'utf8');
  for(const secret of ['fixture-client-secret','fixture-browser-access-token','fixture-browser-refresh-token','code=','state='])assert(!logs.includes(secret),`server logs leaked ${secret}`);
  console.log('PASS: Chromium same-window OAuth redirect, PKCE callback, connected status, reauthorization keeps credential, metadata/log secrecy');
} finally {
  if (runner && runner.exitCode===null) {runner.kill();await new Promise(r=>runner.once("exit",r));}
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  provider.closeAllConnections();await new Promise(r=>provider.close(r));
  await log.close();
  await rm(work, { recursive: true, force: true });
}
