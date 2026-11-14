import { startBlank } from './ui-template-chooser.mjs';
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
  const context = await browser.newContext({permissions:['clipboard-read','clipboard-write'], locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, viewport: { width: 1280, height: 800 } });
  const errors = [];

  const headers = { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' };
  const post = async (ctx, path, data) => {
    const response = await ctx.request.post(`${origin}/api${path}`, { headers, data });
    assert(response.ok(), `${path}: ${await response.text()}`); return response.json();
  };
  await post(context, '/setup', {email:'admin@example.invalid',name:'Admin',password:'temporary-admin-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const owner = await browser.newContext({permissions:['clipboard-read','clipboard-write'], locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' } });
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
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/new`);
  await startBlank(p);
  await p.getByRole('textbox',{name:/^名称/}).and(p.locator('[name="name"]')).fill('Visual application');
  await p.getByRole('combobox',{name:'目标主机',exact:true}).click();
  await p.getByRole('option',{name:'Manifest host',exact:true}).click();
  // The application definition is a schema-driven field tree (ApplicationDefinitionFields): optional keys are added
  // through each object's "添加字段" picker and array items through their "添加…" button.
  const editor = p.getByRole('region',{name:'应用定义',exact:true});
  const sourceToggle = name => editor.getByRole('button',{name,exact:true});
  const services = editor.getByRole('region',{name:'服务配置',exact:true});
  const service = index => services.locator(':scope > div > article').nth(index-1);
  const sub = (scope,label) => scope.locator(`:scope > div > section[aria-label="${label}"]`);
  const item = (scope,index) => scope.locator(':scope > div > article').nth(index-1);
  const own = (scope,role,name) => scope.locator(':scope > div > dl').getByRole(role,{name,exact:true});
  const addField = async (scope,label) => {
    const add = scope.locator(':scope > div > [data-definition-add], :scope > [data-definition-add]').first();
    await add.getByRole('combobox').selectOption({label});
    await add.getByRole('button',{name:'添加字段',exact:true}).click();
  };
  const removeItem = async scope => {
    await scope.locator(':scope > div').first().getByRole('button',{name:'删除',exact:true}).click();
    await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
  };
  const source = editor.getByRole('textbox',{name:'应用配置',exact:true});
  assert.equal(await source.count(),0);
  assert.equal(await editor.locator('[draggable="true"]').count(),0);
  assert.equal(await editor.getByRole('button',{name:/上移|下移|拖动/}).count(),0);
  await editor.getByRole('region',{name:'定义信息',exact:true}).getByRole('textbox',{name:'名称',exact:true}).fill('visual-app');
  await own(service(1),'textbox','镜像').fill('nginx:stable');
  await own(item(sub(service(1),'端口'),1),'spinbutton','主机发布端口').fill('8090');
  await addField(service(1),'环境变量');
  await sub(service(1),'环境变量').getByRole('button',{name:'添加环境变量',exact:true}).click();
  const env = item(sub(service(1),'环境变量'),1);
  await own(env,'textbox','名称').fill('TOKEN');
  await addField(env,'变量引用'); // a reference replaces the plain value
  await own(env,'textbox','变量引用').fill('variable/token');
  await addField(service(1),'数据卷');
  await sub(service(1),'数据卷').getByRole('button',{name:'添加数据卷',exact:true}).click();
  const volume = item(sub(service(1),'数据卷'),1);
  await own(volume,'textbox','名称').fill('data');
  await own(volume,'textbox','挂载绝对路径').fill('/var/data');
  await addField(service(1),'配置文件');
  await sub(service(1),'配置文件').getByRole('button',{name:'添加配置文件',exact:true}).click();
  const file = item(sub(service(1),'配置文件'),1);
  await own(file,'textbox','文件绝对路径').fill('/etc/app.conf');
  await own(file,'textbox','配置文件模板 ID 或名称').fill(config.id);
  await addField(service(1),'资源限制');
  await addField(sub(service(1),'资源限制'),'上限');
  const limits = sub(sub(service(1),'资源限制'),'上限');
  await addField(limits,'CPU');await own(limits,'textbox','CPU').fill('500m');
  await addField(limits,'内存');await own(limits,'textbox','内存').fill('256Mi');
  await addField(service(1),'健康监控');
  const health = sub(service(1),'健康监控');
  await addField(health,'就绪检查');
  // Readiness probes the service's container port (80 for the default web service).
  await own(sub(sub(health,'就绪检查'),'HTTP GET 探测'),'spinbutton','容器／服务端口').fill('80');
  await services.getByRole('button',{name:'添加服务配置',exact:true}).click();
  await own(service(2),'textbox','名称').fill('cache');
  await own(service(2),'textbox','镜像').fill('redis:7');
  await addField(service(1),'依赖服务');
  await sub(service(1),'依赖服务').getByRole('button',{name:'添加依赖服务',exact:true}).click();
  await sub(service(1),'依赖服务').locator('textarea').first().fill('cache');
  // Renaming a service rewrites the dependency references to it.
  await own(service(2),'textbox','名称').fill('redis');
  await sourceToggle('源码编辑').click();
  let manifest = parse(await readEditor(p,source));
  assert.deepEqual(manifest.services.map(s=>s.name),['web','redis']);
  assert.deepEqual(manifest.services[0].dependsOn,['redis']);
  assert.equal(manifest.services[0].env[0].valueFrom,'variable/token');
  assert.equal(manifest.services[0].health.readiness.httpGet.port,80);
  assert.equal(manifest.services[0].resources.limits.cpu,'500m');
  assert.equal(manifest.services[0].ports[0].published,8090);
  const beforeInvalid = await readEditor(p,source);
  await writeEditor(p,source,'services: [');
  for(let attempt=0;attempt<50&&!(await p.getByRole('button',{name:'创建',exact:true}).isDisabled());attempt++)await new Promise(r=>setTimeout(r,50)); assert(await p.getByRole('button',{name:'创建',exact:true}).isDisabled());
  // Malformed source cannot be switched to the form, and stays as typed.
  assert(await sourceToggle('表单编辑').isDisabled());
  assert.equal(await readEditor(p,source),'services: [');
  await writeEditor(p,source,beforeInvalid);
  await sourceToggle('表单编辑').click();
  await removeItem(service(1));
  await sourceToggle('源码编辑').click();
  // Restore the explicitly deleted service, then import advanced fields.
  manifest.metadata.labels = { release:'01' };
  manifest.overrides = {compose:{services:{web:{restart:'unless-stopped'}}}};
  await writeEditor(p,source,'# preserve this comment\n'+JSON.stringify(manifest,null,2));
  await sourceToggle('表单编辑').click();
  await own(service(1),'textbox','镜像').fill('nginx:alpine');
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
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/${created.id}/edit`);
  await own(service(1),'textbox','镜像').fill('nginx:stable');
  // An invalid field blocks both saving and switching to source; correcting it unblocks.
  const published = own(item(sub(service(1),'端口'),1),'spinbutton','主机发布端口');
  await published.fill('');
  await p.waitForFunction(()=>[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='保存')?.disabled);
  assert(await p.getByRole('button',{name:'保存',exact:true}).isDisabled());
  assert(await sourceToggle('源码编辑').isDisabled());
  await published.fill('8090');
  await editor.getByRole('region',{name:'定义信息',exact:true}).getByRole('textbox',{name:'release',exact:true}).fill('02');
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
  console.log('PASS source/form round trip, malformed source/invalid field blocks save, dependency references preserved, edit persists');
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/${created.id}/edit`);
  await own(service(1),'textbox','镜像').waitFor();
  assert.equal(await own(service(1),'textbox','镜像').inputValue(),'nginx:stable');
  await service(1).getByRole('heading',{level:4}).first().evaluate(el=>el.scrollIntoView({block:'start'}));
  await p.screenshot({path:'/tmp/manifest-form-desktop.png',fullPage:false});
  await p.setViewportSize({width:390,height:844});
  await p.screenshot({path:'/tmp/manifest-form-mobile.png',fullPage:false});
  assert(await p.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),'no narrow-screen page overflow');
  await p.setViewportSize({width:1280,height:800});
  const repo = await post(owner,root+'/repositories',{name:'Artifact repository',url:'https://example.invalid/repo.git',default_branch:'main',credential_id:'0'});
  const pipeline = await post(owner,root+'/pipelines',{name:'Artifact pipeline',repository_id:repo.id,target_kind:'host',target_ref:host.id,definition:JSON.stringify({workdir:'/tmp/build/{{run}}',steps:[{name:'build',run:'true'}]})});
  for (const [mode,label] of [['docker','Docker'],['native','Native（systemd）'],['stack','Docker Swarm']]) {
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/new`);
  await startBlank(p);
    await p.getByRole('textbox',{name:/^名称/}).and(p.locator('[name="name"]')).fill(`Form ${mode}`);
    await p.getByRole('combobox',{name:/^部署模式/}).click();
    await p.getByRole('option',{name:label,exact:true}).click();
    const sourceOptions=await service(1).getByLabel('运行来源',{exact:true}).locator('option:not([disabled])').evaluateAll(options=>options.map(option=>option.value));
    assert(sourceOptions.includes(mode==='native'?'artifact':'image'),'target-compatible run source is offered');
    assert(!sourceOptions.includes(mode==='native'?'image':'artifact'),'incompatible run source cannot be selected');
    await p.getByRole('combobox',{name:'目标主机',exact:true}).click();
    await p.getByRole('option',{name:'Manifest host',exact:true}).click();
    if(mode==='native') {
      await service(1).getByLabel('运行来源',{exact:true}).selectOption('artifact');
      const artifact = sub(service(1),'流水线构建产物（Native）');
      // The artifact source names its pipeline; the manifest still stores the ID.
      await artifact.getByRole('combobox',{name:'流水线 ID',exact:true}).click();
      await p.getByRole('option',{name:'Artifact pipeline',exact:true}).click();
      // The build picker loads for the chosen pipeline and defaults to the latest successful build.
      await addField(artifact,'固定构建执行 ID');
      await artifact.getByRole('combobox',{name:'固定构建',exact:true}).filter({hasText:'最新成功构建'}).waitFor();
      await own(artifact,'textbox','产物相对路径').fill('bin/server');
      await removeItem(item(sub(service(1),'端口'),1));
      // The picker's "最新成功构建" default stores an empty run, which the server accepts as "latest".
    }
    if(mode==='stack') {
      await addField(editor.locator('section[data-application-definition]'),'部署策略');
      const strategy = editor.getByRole('region',{name:'部署策略',exact:true});
      await addField(strategy,'类型');await addField(strategy,'最大不可用副本数');
      await strategy.getByRole('combobox',{name:'类型',exact:true}).selectOption({label:'滚动更新'});
      await strategy.getByRole('spinbutton',{name:'最大不可用副本数',exact:true}).fill('0');
    }
    const saving = p.waitForResponse(r=>r.url().endsWith('/deployments')&&r.request().method()==='POST');
    await p.getByRole('button',{name:'创建',exact:true}).click();
    const response = await saving; assert.equal(response.status(),201,await response.text());
    const app = await response.json(); const row = await get(root+'/deployments/'+app.id);
    assert.equal(row.mode,mode);
    if(mode==='native') { const {run,...stored}=parse(row.manifest).services[0].artifact; assert(!run,'default pin means latest successful build'); assert.deepEqual(stored,{source:'pipeline',pipeline:pipeline.id,path:'bin/server'}); }
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

async function focusEditor(locator){await locator.locator('xpath=ancestor::div[contains(@class,"monaco-editor")][1]').locator('.view-lines').click();}
async function readEditor(page,locator){await focusEditor(locator);await page.keyboard.press('ControlOrMeta+A');await page.keyboard.press('ControlOrMeta+C');return page.evaluate(()=>navigator.clipboard.readText());}
async function writeEditor(page,locator,value){await focusEditor(locator);await page.keyboard.press('ControlOrMeta+A');await page.evaluate(value=>navigator.clipboard.writeText(value),value);await page.keyboard.press('ControlOrMeta+V');}
