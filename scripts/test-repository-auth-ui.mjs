/** Real Chromium platform connection -> private Git checkout -> build acceptance. */
import assert from 'node:assert/strict';
import { createServer as createHttpServer } from 'node:http';
import { createHash, randomUUID } from 'node:crypto';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, open, mkdir, writeFile } from 'node:fs/promises';
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
    response.writeHead(200,{'Content-Type':'text/html','Cross-Origin-Opener-Policy':'same-origin'});response.end(`<script>location.href=${JSON.stringify(callback.href)}</script>`);return;
  }
  if(url.pathname==='/login/oauth/access_token') {
    let body='';for await(const part of request)body+=part;
    const form=new URLSearchParams(body),auth=codes.get(form.get('code'));
    assert(auth);codes.delete(form.get('code'));
    assert.equal(createHash('sha256').update(form.get('code_verifier')).digest('base64url'),auth.get('code_challenge'));
    assert.equal(form.get('client_secret'),'fixture-client-secret');
    response.setHeader('Content-Type','application/json');response.end(JSON.stringify({access_token:'fixture-browser-access-token',refresh_token:'fixture-browser-refresh-token',expires_in:3600}));return;
  }
  if(url.pathname.startsWith('/team/test-repo.git/')) {
    const expected='Basic '+Buffer.from('browser-authorized-user:valid-personal-token').toString('base64');
    if(request.headers.authorization!==expected) {response.writeHead(401,{'WWW-Authenticate':'Basic realm="private"'});response.end();return;}
    const path=url.pathname.slice('/team/test-repo.git/'.length);
    if(path.includes('..')) {response.writeHead(400);response.end();return;}
    try {response.end(await readFile(join(work,'repository.git',path)));}
    catch {response.writeHead(404);response.end();}
    return;
  }
  const token=url.searchParams.get('access_token') || request.headers['private-token'] || request.headers['x-yunxiao-token'] || (request.headers.authorization ?? '').replace(/^(Bearer|token) /,'');
  if (!['fixture-browser-access-token','valid-personal-token'].includes(token)) {response.writeHead(401);response.end();return;}
  response.setHeader('Content-Type','application/json');
  if (url.pathname.endsWith('/user')) response.end(JSON.stringify({login:'browser-authorized-user'}));
  else if (url.pathname.endsWith('/repos') || url.pathname.endsWith('/projects') || url.pathname.endsWith('/repositories')) response.end(JSON.stringify([{name:'repo-'.repeat(15),full_name:'team/test-repo',clone_url:providerOrigin+'/team/test-repo.git',default_branch:'develop'}]));
  else {response.writeHead(404);response.end();}

});
await new Promise(r=>provider.listen(0,'127.0.0.1',r));
const providerOrigin=`http://127.0.0.1:${provider.address().port}`;
const work = await mkdtemp(join(tmpdir(), 'platform-ui-'));
const seed=join(work,'seed');await mkdir(seed);
await writeFile(join(seed,'build.sh'),'set -eu\nmkdir -p output\nprintf "built-from-private-repository\\n" > output/result.txt\ngit rev-parse HEAD >> output/result.txt\n');
const git=(...args)=>execFileSync('git',['-C',seed,...args],{windowsHide:true,env:{...process.env,GIT_AUTHOR_NAME:'fixture',GIT_AUTHOR_EMAIL:'fixture@example.invalid',GIT_COMMITTER_NAME:'fixture',GIT_COMMITTER_EMAIL:'fixture@example.invalid'}});
git('init','--quiet','--initial-branch','develop');git('add','build.sh');git('commit','--quiet','-m','fixture');
const sourceCommit=git('rev-parse','HEAD').toString().trim();
execFileSync('git',['clone','--bare','--quiet',seed,join(work,'repository.git')],{windowsHide:true});
execFileSync('git',['--git-dir='+join(work,'repository.git'),'update-server-info'],{windowsHide:true});
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}` };
delete env.FERRY_DATABASE_URL;
for (const key of Object.keys(env)) if (key.startsWith('REPOSITORY_OAUTH_')) delete env[key];
env.RUST_LOG='ferry_server=debug';
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
  const platform=await context.newPage();platform.on('pageerror',e=>errors.push(e.message));
  await platform.goto(`${origin}/_platform/settings/1/edit`);
  await platform.getByLabel('平台公开访问地址',{exact:true}).fill(origin);
  await platform.getByLabel('启用 GitHub 网页授权',{exact:true}).check();
  await platform.getByLabel('GitHub 应用 ID',{exact:true}).fill('fixture-client-id');
  await platform.getByLabel('替换GitHub 应用密钥（不勾选则保留原值）',{exact:true}).check();
  await platform.getByLabel('GitHub 应用密钥',{exact:true}).fill('fixture-client-secret');
  await platform.getByLabel('GitHub 网站地址',{exact:true}).fill(providerOrigin);
  await platform.getByLabel('GitHub API 地址',{exact:true}).fill(providerOrigin);
  await platform.getByRole('button',{name:'保存',exact:true}).click();
  await platform.getByText('平台设置已保存，立即生效。',{exact:true}).waitFor();
  await platform.getByLabel('GitHub 应用 ID',{exact:true}).waitFor();
  assert.equal(await platform.getByLabel('GitHub 应用密钥',{exact:true}).count(),0,'stored app secret is not returned');
  assert.equal(await platform.getByLabel('当前回调地址',{exact:true}).inputValue(),origin+'/api/repository-auth/callback');

  await p.goto(`${origin}/tenant/build/repositories`);
  assert.equal(await p.getByRole('navigation',{name:'仓库页面',exact:true}).count(),0,'no separate repository authorization tabs');
  assert.equal(await p.getByRole('link',{name:'平台授权',exact:true}).count(),0);
  await p.goto(`${origin}/tenant/build/repositories/new`);
  await p.getByLabel('代码来源',{exact:true}).selectOption('github');
  await p.getByLabel(/^名称/).fill('Preserved draft');
  await p.getByLabel(/^备注/).fill('Keep this during OAuth');
  const popupPromise=p.waitForEvent('popup');
  await p.getByRole('button',{name:'打开网页授权',exact:true}).click();
  const popup=await popupPromise;
  await p.getByText('已连接：browser-authorized-user',{exact:true}).waitFor();
  assert.equal(await p.getByLabel(/^名称/).inputValue(),'Preserved draft');
  assert.equal(await p.getByLabel(/^备注/).inputValue(),'Keep this during OAuth');
  assert(popup.isClosed(),'OAuth callback closes only its popup');
  await p.getByLabel('选择平台仓库',{exact:true}).selectOption(providerOrigin+'/team/test-repo.git');
  assert.equal(await p.getByLabel(/^默认分支/).inputValue(),'develop');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL('**/tenant/build/repositories');
  let rows=await (await owner.request.get(`${origin}/api${root}/repositories`)).json();
  assert.equal(rows.items[0].name,'Preserved draft');assert.equal(rows.items[0].provider,'github');assert.notEqual(rows.items[0].credential_id,'0');
  const firstCredential=rows.items[0].credential_id;

  // Reauthorization uses the same connection and preserves the still-unsaved form.
  await p.goto(`${origin}/tenant/build/repositories/new`);
  await p.getByLabel('代码来源',{exact:true}).selectOption('github');
  await p.getByRole('button',{name:'打开网页授权',exact:true}).click();
  await p.getByText('已连接：browser-authorized-user',{exact:true}).waitFor();
  const connections=await (await owner.request.get(`${origin}/api${root}/repository-auth`)).json();
  assert.equal(connections.items.find(row=>row.id==='github').credential_id,firstCredential);

  await p.getByLabel('代码来源',{exact:true}).selectOption('gitee');
  assert.equal(await p.getByLabel('选择平台仓库',{exact:true}).count(),0,'changing platform clears old repository selection');
  await p.getByLabel('平台授权方式',{exact:true}).selectOption('token');
  await p.getByLabel('个人访问令牌',{exact:true}).fill('invalid-token');
  await p.getByText('自定义接入点（自建平台时填写）',{exact:true}).click();
  await p.getByLabel('平台网站地址',{exact:true}).fill(providerOrigin);
  await p.getByLabel('平台 API 地址',{exact:true}).fill(providerOrigin);
  await p.getByRole('button',{name:'连接并读取仓库',exact:true}).click();
  await p.getByRole('alert').filter({hasText:'HTTP 401'}).waitFor();
  assert(await p.getByRole('button',{name:'创建',exact:true}).isDisabled());
  assert.equal(await p.getByLabel('选择平台仓库',{exact:true}).count(),0);
  await p.getByLabel('个人访问令牌',{exact:true}).fill('valid-personal-token');
  await p.getByRole('button',{name:'连接并读取仓库',exact:true}).click();
  await p.getByLabel('选择平台仓库',{exact:true}).selectOption(providerOrigin+'/team/test-repo.git');
  assert.equal(await p.getByLabel('个人访问令牌',{exact:true}).inputValue(),'','token cleared after successful exchange');
  await p.getByLabel(/^名称/).fill('Gitee connected repository');
  await p.setViewportSize({width:390,height:844});
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'repository form fits mobile viewport');
  if(process.env.UI_SCREENSHOT) await p.screenshot({path:process.env.UI_SCREENSHOT,fullPage:true});
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL('**/tenant/build/repositories');
  rows=await (await owner.request.get(`${origin}/api${root}/repositories`)).json();
  assert(rows.items.some(row=>row.provider==='gitee'&&row.name==='Gitee connected repository'&&row.credential_id!=='0'));

  await p.goto(`${origin}/tenant/build/repositories/new`);
  await p.getByLabel('代码来源',{exact:true}).selectOption('git');
  await p.getByLabel('Git 仓库地址',{exact:true}).fill(providerOrigin+'/generic.git');
  await p.getByLabel('Git 授权方式',{exact:true}).selectOption('password');
  await p.getByLabel('Git 账号',{exact:true}).fill('git-user');
  await p.getByLabel('Git 密码',{exact:true}).fill('generic-private-password');
  await p.getByLabel(/^名称/).fill('Generic with password');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL('**/tenant/build/repositories');
  rows=await (await owner.request.get(`${origin}/api${root}/repositories`)).json();
  assert(rows.items.some(row=>row.provider==='git'&&row.name==='Generic with password'&&row.credential_id!=='0'));

  const buildHost=await post(owner,root+'/hosts',{name:'Connected build host',connection:'runner'});
  const runnerToken=await post(owner,root+'/runner-tokens',{host_id:buildHost.id});
  runner=spawn(resolve('target/debug/ferry-runner'),['--server',origin,'--token',runnerToken.token,'--state-dir',join(work,'runner')],{stdio:'ignore',windowsHide:true});
  for(let i=0;;i++) {
    const host=await (await owner.request.get(origin+'/api'+root+'/hosts/'+buildHost.id)).json();
    if(host.state==='online')break;
    assert(i<100,'runner connection timed out');await new Promise(r=>setTimeout(r,100));
  }
  await p.goto(origin+'/tenant/build/pipelines/new');
  await p.getByLabel(/^名称/).fill('Build from connection');
  await p.getByLabel('构建代码来源',{exact:true}).selectOption('connection');
  assert.equal(await p.getByLabel('代码来源',{exact:true}).locator('option[value=git]').count(),0,'connection source only lists platforms; generic Git uses saved repository');
  await p.getByLabel('代码来源',{exact:true}).selectOption('gitee');
  await p.getByRole('button',{name:/使用已有连接/}).click();
  await p.getByLabel('选择平台仓库',{exact:true}).selectOption(providerOrigin+'/team/test-repo.git');
  assert.equal(await p.getByLabel(/^名称/).inputValue(),'Build from connection');
  const definition=JSON.parse(await p.getByLabel(/^定义（JSON）/).inputValue());
  assert.equal(definition.steps[0].checkout,true);
  assert.equal(definition.steps[1].run,'echo hello');
  definition.workdir=join(work,'build-{{run}}');
  definition.steps[1].run='sh build.sh';definition.artifacts=['output/result.txt'];
  await p.getByLabel(/^定义（JSON）/).fill(JSON.stringify(definition));
  await p.getByRole('combobox',{name:/目标主机/}).click();
  await p.getByRole('option',{name:'Connected build host',exact:true}).click();
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL('**/tenant/build/pipelines');
  const builds=await (await owner.request.get(origin+'/api'+root+'/pipelines')).json();
  const build=builds.items.find(row=>row.name==='Build from connection');
  assert(build);
  const bound=await (await owner.request.get(origin+'/api'+root+'/repositories/'+build.repository_id)).json();
  assert.equal(bound.provider,'gitee');assert.equal(bound.url,providerOrigin+'/team/test-repo.git');
  const run=await post(owner,root+'/pipelines/'+build.id+'/actions/run',{});
  let result;
  for(let i=0;;i++) {
    result=await (await owner.request.get(origin+'/api'+root+'/pipeline-runs/'+run.id)).json();
    if(!['queued','running'].includes(result.state))break;
    assert(i<200,'build timed out');await new Promise(r=>setTimeout(r,100));
  }
  assert.equal(result.state,'succeeded',JSON.stringify(result.steps));
  assert.equal(await readFile(join(work,'build-'+result.number,'output/result.txt'),'utf8'),'built-from-private-repository\n'+sourceCommit+'\n');
  assert.equal(result.artifacts.length,1);
  console.log('PASS: browser-selected connection -> authenticated private Git checkout -> checked-in build script -> exact commit artifact');
  console.log('PASS: build form browses existing connection, binds selected platform repository and preserves commands/name with explicit checkout');
  console.log('PASS: dynamic platform settings, secret nonreturn, popup OAuth preserves draft, Gitee PAT validates and binds selected repository, no auth tabs, generic Git inline credentials, mobile form');
  assert.deepEqual(errors,[]);
  const logs=await readFile(join(work,'server.log'),'utf8');
  for(const secret of ['fixture-client-secret','fixture-browser-access-token','fixture-browser-refresh-token','code=','state='])assert(!logs.includes(secret),`server logs leaked ${secret}`);
  console.log('PASS: reauthorization retains references and secrets stay out of metadata/logs');
} catch (error) {
  for (const context of browser?.contexts() ?? []) for (const page of context.pages()) {
    console.error('PAGE',page.url(),await page.locator('body').innerText().catch(()=>''));
    console.error('SELECTS',await page.locator('select').evaluateAll(nodes=>nodes.map(node=>({html:node.outerHTML,label:node.labels?.[0]?.textContent}))));
  }
  throw error;
} finally {
  if (runner && runner.exitCode===null) {runner.kill();await new Promise(r=>runner.once("exit",r));}
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  provider.closeAllConnections();await new Promise(r=>provider.close(r));
  await log.close();
  await rm(work, { recursive: true, force: true });
}
