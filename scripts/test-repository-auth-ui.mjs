/** Real Chromium platform connection -> private Git checkout -> build acceptance. */
import assert from 'node:assert/strict';
import { parseDocument } from 'yaml';
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
  if (url.pathname.endsWith('/branches')) response.end(JSON.stringify([{name:'develop'},{name:'release'}]));
  else if (url.pathname.endsWith('/user')) response.end(JSON.stringify({login:'browser-authorized-user'}));
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
  await p.getByLabel('代码平台',{exact:true}).selectOption('github');
  await p.getByLabel(/^连接名称/).fill('Preserved draft');
  await p.getByLabel(/^备注/).fill('Keep this during OAuth');
  assert.equal(await p.getByLabel(/^默认分支/).count(),0,'connections do not own a default branch');
  assert.equal(await p.getByLabel('选择平台仓库',{exact:true}).count(),0,'connections do not select one repository');
  await p.waitForFunction(()=>document.querySelector('select[aria-label="平台授权方式"] option[value="web"]')?.disabled===false);
  await p.getByLabel('平台授权方式',{exact:true}).selectOption('web');
  const popupPromise=p.waitForEvent('popup');
  await p.getByRole('button',{name:'打开网页授权',exact:true}).click();
  const popup=await popupPromise;
  await p.waitForURL(url=>/^\/tenant\/build\/repositories\/[^/]+$/.test(url.pathname)&&!url.pathname.endsWith('/new'));
  if(!popup.isClosed())await popup.waitForEvent('close');
  assert(popup.isClosed(),'OAuth callback closes only its popup');
  let rows=await (await owner.request.get(`${origin}/api${root}/code-connections`)).json();
  assert.equal(rows.items.length,1,'OAuth must create only one named connection without a second form submit');
  const oauthConnection=rows.items[0];
  assert.equal(oauthConnection.name,'Preserved draft');assert.equal(oauthConnection.description,'Keep this during OAuth');assert.equal(oauthConnection.provider,'github');assert.notEqual(oauthConnection.credential_id,'0');
  assert.equal(new URL(p.url()).pathname,`/tenant/build/repositories/${oauthConnection.id}`);
  const firstCredential=oauthConnection.credential_id;

  // Reauthorization updates this named connection, preserving its ID and credential references.
  await p.goto(`${origin}/tenant/build/repositories/${oauthConnection.id}/edit`);
  await p.getByLabel(/^连接名称/).fill('Reauthorized connection');
  await p.getByLabel(/^备注/).fill('Saved during reauthorization');
  const reauthPopupPromise=p.waitForEvent('popup');
  await p.getByRole('button',{name:'重新网页授权',exact:true}).click();
  const reauthPopup=await reauthPopupPromise;
  await p.waitForURL(`**/tenant/build/repositories/${oauthConnection.id}`);
  if(!reauthPopup.isClosed())await reauthPopup.waitForEvent('close');
  assert(reauthPopup.isClosed());
  rows=await (await owner.request.get(`${origin}/api${root}/code-connections`)).json();
  assert.equal(rows.items.length,1);assert.equal(rows.items[0].id,oauthConnection.id);assert.equal(rows.items[0].credential_id,firstCredential);
  assert.equal(rows.items[0].name,'Reauthorized connection');assert.equal(rows.items[0].description,'Saved during reauthorization');

  const invalidCredential=await post(owner,root+'/credentials',{name:'Invalid platform PAT',kind:'token',username:'browser-authorized-user',secret:'invalid-token',description:''});
  await p.goto(`${origin}/tenant/build/repositories/new`);
  await p.getByLabel('代码平台',{exact:true}).selectOption('gitee');
  await p.getByLabel(/^连接名称/).fill('Gitee connection');
  await p.getByLabel('授权认证',{exact:true}).selectOption(invalidCredential.id);
  assert.equal(await p.locator('input[type="password"]').count(),0,'connection form never copies credential secrets');
  await p.getByText('平台接入地址（自建平台时填写）',{exact:true}).click();
  await p.getByLabel('平台网站地址',{exact:true}).fill(providerOrigin);
  await p.getByLabel('平台 API 地址',{exact:true}).fill(providerOrigin);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByText(/HTTP 401/).waitFor();
  rows=await (await owner.request.get(`${origin}/api${root}/code-connections`)).json();assert.equal(rows.items.length,1,'failed PAT validation must not create a connection');
  assert.equal(await p.getByLabel('选择平台仓库',{exact:true}).count(),0);

  // The shared authorization form opens separately; the connection draft survives.
  const credentialPopupPromise=p.waitForEvent('popup');
  await p.getByRole('link',{name:'新建授权认证',exact:true}).click();
  const credentialPage=await credentialPopupPromise;credentialPage.on('pageerror',error=>errors.push(error.message));
  await credentialPage.getByLabel(/^名称/).fill('Reusable Gitee PAT');
  await credentialPage.getByRole('combobox',{name:/类型/}).click();await credentialPage.getByRole('option',{name:'访问令牌',exact:true}).click();
  await credentialPage.getByLabel(/^账号/).fill('browser-authorized-user');
  await credentialPage.getByLabel(/^内容/).fill('valid-personal-token');
  await credentialPage.getByRole('button',{name:'创建',exact:true}).click();await credentialPage.waitForURL('**/tenant/build/credentials');
  const credentialRows=await (await owner.request.get(`${origin}/api${root}/credentials`)).json();
  const personalCredential=credentialRows.items.find(row=>row.name==='Reusable Gitee PAT');assert(personalCredential);assert(!('secret' in personalCredential));assert(personalCredential.has_secret);
  await credentialPage.close();
  assert.equal(await p.getByLabel(/^连接名称/).inputValue(),'Gitee connection');assert.equal(await p.getByLabel('平台网站地址',{exact:true}).inputValue(),providerOrigin);
  await p.getByRole('button',{name:'刷新授权认证',exact:true}).click();
  await p.getByLabel('授权认证',{exact:true}).selectOption(personalCredential.id);
  await p.setViewportSize({width:390,height:844});
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'connection form fits mobile viewport');
  if(process.env.UI_SCREENSHOT) await p.screenshot({path:process.env.UI_SCREENSHOT,fullPage:true});
  const saveConnection=p.waitForResponse(response=>response.url()===`${origin}/api${root}/code-connections`&&response.request().method()==='POST');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  const savedResponse=await saveConnection;assert(savedResponse.ok(),await savedResponse.text());
  const savedBody=savedResponse.request().postDataJSON();assert.equal(savedBody.credential_id,personalCredential.id);assert(!('token' in savedBody)&&!('secret' in savedBody));assert(!('default_branch' in savedBody));
  await p.waitForURL('**/tenant/build/repositories');
  rows=await (await owner.request.get(`${origin}/api${root}/code-connections`)).json();
  const giteeConnection=rows.items.find(row=>row.provider==='gitee'&&row.name==='Gitee connection');assert(giteeConnection);assert.equal(giteeConnection.credential_id,personalCredential.id);assert(!('default_branch' in giteeConnection));
  await p.goto(`${origin}/tenant/build/repositories/${giteeConnection.id}/edit`);assert.equal(await p.getByLabel('授权认证',{exact:true}).inputValue(),personalCredential.id);assert.equal(await p.getByLabel('平台网站地址',{exact:true}).inputValue(),new URL(providerOrigin).href);

  const passwordCredential=await post(owner,root+'/credentials',{name:'Reusable Git password',kind:'password',username:'git-user',secret:'generic-private-password',description:''});
  await p.goto(`${origin}/tenant/build/repositories/new`);
  await p.getByLabel('代码平台',{exact:true}).selectOption('git');
  await p.getByLabel('Git 仓库地址（可选）',{exact:true}).fill(providerOrigin+'/generic.git');
  await p.getByLabel('授权认证',{exact:true}).selectOption(passwordCredential.id);
  await p.getByLabel(/^连接名称/).fill('Generic with password');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL('**/tenant/build/repositories');
  rows=await (await owner.request.get(`${origin}/api${root}/code-connections`)).json();
  assert(rows.items.some(row=>row.provider==='git'&&row.name==='Generic with password'&&row.credential_id===passwordCredential.id));

  const buildHost=await post(owner,root+'/hosts',{name:'Connected build host',connection:'runner'});
  const runnerToken=await post(owner,root+'/runner-tokens',{host_id:buildHost.id});
  runner=spawn(resolve('target/debug/ferry-runner'),['--server',origin,'--token',runnerToken.token,'--state-dir',join(work,'runner')],{stdio:'ignore',windowsHide:true});
  for(let i=0;;i++) {
    const host=await (await owner.request.get(origin+'/api'+root+'/hosts/'+buildHost.id)).json();
    if(host.state==='online')break;
    assert(i<100,'runner connection timed out');await new Promise(r=>setTimeout(r,100));
  }
  await p.setViewportSize({width:1280,height:900});
  await p.goto(origin+'/tenant/build/pipelines/new');
  await p.getByLabel(/^名称/).fill('Build from connection');
  assert.equal(await p.getByLabel('构建代码来源',{exact:true}).count(),0);
  assert.equal(await p.getByRole('combobox',{name:/目标主机/}).count(),0);
  await p.getByRole('button',{name:'构建',exact:true}).first().click();
  await p.getByLabel('任务执行器',{exact:true}).selectOption(`host:${buildHost.id}`);
  await p.getByRole('button',{name:'添加拉取代码步骤',exact:true}).click();
  await p.getByLabel('仓库连接',{exact:true}).selectOption(giteeConnection.id);
  await p.getByLabel('节点仓库',{exact:true}).selectOption(providerOrigin+'/team/test-repo.git');
  assert.equal(await p.getByLabel('分支 / 标签 / Commit',{exact:true}).inputValue(),'develop');
  await p.locator('datalist option[value="develop"]').waitFor({state:'attached'});
  await p.getByLabel('分支 / 标签 / Commit',{exact:true}).fill(sourceCommit);
  assert.equal(await p.getByLabel(/^名称/).inputValue(),'Build from connection');
  await p.getByRole('tab',{name:'YAML',exact:true}).click();
  const definition=parseDocument(await p.getByLabel('GitHub Actions YAML',{exact:true}).inputValue());
  const steps=definition.toJS().jobs.build.steps;
  const checkout=steps.find(step=>step.uses==='actions/checkout@v4'),command=steps.find(step=>step.run);
  assert(checkout?.id,'checkout configuration assigns a stable step ID');assert.equal(command.run,'echo hello');
  definition.setIn(['jobs','build','steps'],[checkout,{...command,run:'sh build.sh'},{uses:'actions/upload-artifact@v4',with:{name:'result',path:'output/result.txt','if-no-files-found':'error'}}]);
  await p.getByLabel('GitHub Actions YAML',{exact:true}).fill(definition.toString());
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.waitForURL('**/tenant/build/pipelines');
  const builds=await (await owner.request.get(origin+'/api'+root+'/pipelines')).json();
  const build=builds.items.find(row=>row.name==='Build from connection');assert(build);
  const binding=build.checkout_bindings.find(row=>row.job_id==='build'&&row.step_id===checkout.id);assert(binding);assert.equal(binding.connection_id,giteeConnection.id);assert.equal(binding.repository_url,providerOrigin+'/team/test-repo.git');assert.equal(binding.ref,sourceCommit);
  assert.equal(build.executor_bindings.find(row=>row.job_id==='build').target_ref,buildHost.id);
  await p.goto(origin+'/tenant/build/pipelines/'+build.id+'/edit');
  await p.locator('.react-flow__node[data-id="build"]').getByRole('button',{name:/拉取代码/}).click();
  assert.equal(await p.getByLabel('仓库连接',{exact:true}).inputValue(),giteeConnection.id);assert.equal(await p.getByLabel('节点仓库',{exact:true}).inputValue(),providerOrigin+'/team/test-repo.git');assert.equal(await p.getByLabel('分支 / 标签 / Commit',{exact:true}).inputValue(),sourceCommit);
  const run=await post(owner,root+'/pipelines/'+build.id+'/actions/run',{});
  let result;
  for(let i=0;;i++) {
    result=await (await owner.request.get(origin+'/api'+root+'/pipeline-runs/'+run.id)).json();
    if(!['queued','running'].includes(result.state))break;
    assert(i<1200,'build timed out');await new Promise(r=>setTimeout(r,100));
  }
  assert.equal(result.state,'succeeded',JSON.stringify(result.steps));
  assert.equal(result.artifacts.length,1);
  const archive=await owner.request.get(origin+'/api'+root+'/pipeline-artifacts/'+result.artifacts[0].id+'/download');
  assert(archive.ok());
  const content=execFileSync('python3',['-c','import io,sys,zipfile;sys.stdout.buffer.write(zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())).read("result.txt"))'],{input:await archive.body(),windowsHide:true}).toString();
  assert.equal(content,'built-from-private-repository\n'+sourceCommit+'\n');
  console.log('PASS: browser-selected connection -> authenticated private Git checkout -> checked-in build script -> exact commit artifact');
  console.log('PASS: checkout node browses named connection/repository/branch, persists stable binding and task executor through save/reload');
  console.log('PASS: dynamic platform settings, secret nonreturn, named popup OAuth/reauthorization, shared PAT/password credentials, new-tab credential creation preserves draft, no default branch on connections, mobile form');
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
