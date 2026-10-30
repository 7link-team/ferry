/** Real Chromium -> server acceptance for manifest forms, source round trips and saving. */
import assert from 'node:assert/strict';
import { parse } from 'yaml';
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
  const get=async path => {const r=await owner.request.get(`${origin}/api${path}`); assert(r.ok()); return r.json();};
  const host = await post(owner, root+'/nodes', {name:'Manifest host',connection:'runner'});
  await post(owner, root+'/variables', {name:'token',scope:'deploy',kind:'secret',value:'never-visible-value'});
  const config = await post(owner, root+'/config-templates', {name:'app-conf',content:'enabled=true\n',parameters:'[]'});
  await p.goto(`${origin}/workspaces/${workspace_tenant}/deployments/new`);
  await p.getByRole('textbox',{name:/^名称/}).fill('Visual application');
  await p.getByLabel('目标主机',{exact:false}).click();
  await p.getByRole('option',{name:'Manifest host',exact:true}).click();
  const editor = p.getByRole('region',{name:'应用配置编辑器',exact:true});
  assert.equal(await editor.getByLabel('应用配置源码（YAML / JSON）').count(),0);
  assert.equal(await editor.locator('[draggable="true"]').count(),0);
  assert.equal(await editor.getByRole('button',{name:/上移|下移|拖动/}).count(),0);
  await editor.getByLabel('应用标识',{exact:false}).fill('visual-app');
  const service = index => editor.getByRole('region',{name:`服务 ${index}`,exact:true});
  await service(1).getByLabel('镜像',{exact:false}).fill('nginx:stable');
  await service(1).getByLabel('主机发布端口',{exact:true}).fill('8090');
  await service(1).getByRole('button',{name:'添加环境变量',exact:true}).click();
  await service(1).getByLabel('变量名称',{exact:true}).fill('TOKEN');
  await service(1).getByLabel('取值方式',{exact:true}).selectOption('reference');
  await service(1).getByLabel('变量引用',{exact:false}).fill('variable/token');
  await service(1).getByRole('button',{name:'添加数据卷',exact:true}).click();
  await service(1).getByLabel('卷名称',{exact:true}).fill('data');
  await service(1).getByLabel('挂载绝对路径',{exact:true}).fill('/var/data');
  await service(1).getByRole('button',{name:'添加配置文件',exact:true}).click();
  await service(1).getByLabel('文件绝对路径',{exact:true}).fill('/etc/app.conf');
  await service(1).getByLabel('配置文件模板 ID 或名称',{exact:true}).fill(config.id);
  await service(1).getByText('高级配置：命令、依赖、健康检查与资源限制',{exact:true}).click();
  await service(1).getByLabel('CPU 上限',{exact:false}).fill('500m');
  await service(1).getByLabel('内存上限',{exact:false}).fill('256Mi');
  await service(1).getByRole('checkbox',{name:'就绪检查（HTTP）',exact:true}).check();
  await editor.getByRole('button',{name:'添加服务',exact:true}).click();
  await service(2).getByLabel('服务名称',{exact:true}).fill('cache');
  await service(2).getByLabel('镜像',{exact:false}).fill('redis:7');
  await service(1).getByRole('checkbox',{name:'cache',exact:true}).check();
  await service(2).getByLabel('服务名称',{exact:true}).fill('redis');
  await editor.getByRole('button',{name:'源码编辑',exact:true}).click();
  const source = editor.getByLabel('应用配置源码（YAML / JSON）');
  let manifest = parse(await source.inputValue());
  assert.deepEqual(manifest.services.map(s=>s.name),['web','redis']);
  assert.deepEqual(manifest.services[0].dependsOn,['redis']);
  assert.equal(manifest.services[0].env[0].valueFrom,'variable/token');
  assert.equal(manifest.services[0].health.readiness.httpGet.port,80);
  assert.equal(manifest.services[0].resources.limits.cpu,'500m');
  assert.equal(manifest.services[0].ports[0].published,8090);
  const beforeInvalid = await source.inputValue();
  await source.fill('services: [');
  assert(await p.getByRole('button',{name:'创建',exact:true}).isDisabled());
  await editor.getByRole('button',{name:'表单编辑',exact:true}).click();
  assert.equal(await source.inputValue(),'services: [');
  await source.fill(beforeInvalid);
  await service(1).getByRole('button',{name:'删除服务 web',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除服务',exact:true}).click();
  await editor.getByRole('button',{name:'源码编辑',exact:true}).click();
  // Restore the explicitly deleted service, then import advanced fields.
  manifest.metadata.labels = { release:'01' };
  manifest.overrides = {compose:{services:{web:{restart:'unless-stopped'}}}};
  await source.fill('# preserve this comment\n'+JSON.stringify(manifest,null,2));
  await editor.getByRole('button',{name:'表单编辑',exact:true}).click();
  await service(1).getByLabel('镜像',{exact:false}).fill('nginx:alpine');
  const save = p.waitForResponse(r=>r.url().endsWith('/deployments')&&r.request().method()==='POST');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  const saved = await save; assert.equal(saved.status(),201,await saved.text());
  const created = await saved.json();
  let stored = await get(root+'/deployments/'+created.id);
  assert.equal(stored.target_ref,host.id);
  assert.equal(parse(stored.manifest).services[0].image,'nginx:alpine');
  assert.equal(parse(stored.manifest).metadata.labels.release,'01');
  assert.deepEqual(parse(stored.manifest).overrides,{docker:{},native:{},stack:{},...manifest.overrides});
  assert(saved.request().postDataJSON().manifest.includes('# preserve this comment'));
  console.log('PASS form creates real multi-service deployment with ports, secret references, volumes, files, health/resources and dependencies');
  await p.goto(`${origin}/workspaces/${workspace_tenant}/deployments/${created.id}/edit`);
  await service(1).getByLabel('镜像',{exact:false}).fill('nginx:stable');
  await editor.getByText('应用高级配置：更新策略、标签与模式覆盖',{exact:true}).click();
  await editor.getByLabel('应用标签（JSON 对象）',{exact:true}).fill('{');
  await p.waitForFunction(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='保存')?.disabled);
  assert(await p.getByRole('button',{name:'保存',exact:true}).isDisabled());
  assert(await editor.getByRole('button',{name:'源码编辑',exact:true}).isDisabled());
  await editor.getByLabel('应用标签（JSON 对象）',{exact:true}).fill('{"release":"02"}');
  const update = p.waitForResponse(r=>r.url().endsWith('/deployments/'+created.id)&&r.request().method()==='PATCH');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  const updated = await update; assert.equal(updated.status(),200,await updated.text());
  stored = await get(root+'/deployments/'+created.id);
  const changed = parse(stored.manifest);
  assert.equal(changed.metadata.labels.release,'02');
  assert.equal(changed.services[0].image,'nginx:stable');
  assert.deepEqual(changed.services[0].dependsOn,['redis']);
  assert.deepEqual(changed.overrides,{docker:{},native:{},stack:{},...manifest.overrides});
  assert.equal(typeof stored.manifest,'string'); // server persists its existing canonical JSON format
  console.log('PASS source/form round trip, malformed source/JSON blocks save, dependency references preserved, edit persists');
  await p.goto(`${origin}/workspaces/${workspace_tenant}/deployments/${created.id}/edit`);
  await service(1).getByLabel('镜像',{exact:false}).waitFor();
  assert.equal(await service(1).getByLabel('镜像',{exact:false}).inputValue(),'nginx:stable');
  await service(1).getByRole('heading',{level:3}).evaluate(el=>el.scrollIntoView({block:'start'}));
  await p.screenshot({path:'/tmp/manifest-form-desktop.png',fullPage:false});
  await p.setViewportSize({width:390,height:844});
  await p.screenshot({path:'/tmp/manifest-form-mobile.png',fullPage:false});
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),'no narrow-screen page overflow');
  await p.setViewportSize({width:1280,height:800});
  const repo = await post(owner,root+'/repositories',{name:'Artifact repository',url:'https://example.invalid/repo.git',default_branch:'main',credential_id:'0'});
  const pipeline = await post(owner,root+'/pipelines',{name:'Artifact pipeline',repository_id:repo.id,target_kind:'host',target_ref:host.id,definition:JSON.stringify({workdir:'/tmp/build/{{run}}',steps:[{name:'build',run:'true'}]})});
  for (const [mode,label] of [['docker','Docker'],['native','Native（systemd）'],['stack','Docker Swarm']]) {
    await p.goto(`${origin}/workspaces/${workspace_tenant}/deployments/new`);
    await p.getByRole('textbox',{name:/^名称/}).fill(`Form ${mode}`);
    await p.getByRole('combobox',{name:/^部署模式/}).click();
    await p.getByRole('option',{name:label,exact:true}).click();
    await p.getByLabel('目标主机',{exact:false}).click();
    await p.getByRole('option',{name:'Manifest host',exact:true}).click();
    if(mode==='native') {
      await service(1).getByLabel('运行来源',{exact:true}).selectOption('artifact');
      await service(1).getByLabel('流水线 ID',{exact:true}).fill(pipeline.id);
      await service(1).getByLabel('产物相对路径',{exact:true}).fill('bin/server');
      await service(1).getByRole('button',{name:'删除端口 1',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除端口',exact:true}).click();
    }
    if(mode==='stack') {
      await editor.getByText('应用高级配置：更新策略、标签与模式覆盖',{exact:true}).click();
      await editor.getByLabel('更新策略',{exact:true}).selectOption('rolling');
      await editor.getByLabel('最大不可用副本数',{exact:true}).fill('0');
    }
    const saving = p.waitForResponse(r=>r.url().endsWith('/deployments')&&r.request().method()==='POST');
    await p.getByRole('button',{name:'创建',exact:true}).click();
    const response = await saving; assert.equal(response.status(),201,await response.text());
    const app = await response.json(); const row = await get(root+'/deployments/'+app.id);
    assert.equal(row.mode,mode);
    if(mode==='native') assert.deepEqual(parse(row.manifest).services[0].artifact,{source:'pipeline',pipeline:pipeline.id,path:'bin/server'});
    if(mode==='stack') assert.equal(parse(row.manifest).strategy.type,'rolling');
    console.log(`PASS ${mode} form creates valid configuration through real API`);
  }
  assert.deepEqual(errors,[]);
  console.log('PASS reopened persisted configuration, desktop/narrow-screen layout, no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
