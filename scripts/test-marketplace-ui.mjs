/** Real Chromium marketplace source connection selection, synchronization, import and deletion. */

async function chooseNative(control, value) {
  await control.waitFor();
  await control.page().waitForFunction(({ element, expected }) =>
    [...element.options].some(option => option.value === expected),
  { element: await control.elementHandle(), expected: value });
  const index = await control.evaluate((element, expected) => [...element.options].findIndex(option => option.value === expected), value);
  assert(index >= 0, `Missing option ${value}`);
  await control.click();
  await control.press('Home');
  for (let i = 0; i < index; i++) await control.press('ArrowDown');
  await control.press('Enter');
  assert.equal(await control.inputValue(), value, 'Native selection must persist');
}

import assert from 'node:assert/strict';
import { createServer as createHttpServer } from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, open } from 'node:fs/promises';
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
  const presetOutput = resolve(process.env.FERRY_SCENARIO_UI_OUTPUT || join(work, 'template-ui'));
  await mkdir(presetOutput, { recursive: true });
  for (const width of [1440,900,390,320]) {
    await p.setViewportSize({width,height:900});
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/code-connections/new`);
    await p.getByRole('textbox',{name:/^连接名称/}).fill('Preset Git '+width);
    for (const [label,provider] of [['GitHub 团队构建','github'],['Codeup 企业仓库','codeup'],['纯 Git 拉取','git']]) {
      await p.getByRole('button',{name:label,exact:true}).click();
      assert.equal(await p.getByRole('combobox',{name:'代码平台',exact:true}).inputValue(),provider);
      assert.equal(await p.getByRole('combobox',{name:'凭据',exact:true}).inputValue(),'0');
      assert.equal(await p.getByRole('textbox',{name:/^连接名称/}).inputValue(),'Preset Git '+width);
    }
    assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Repository preset page overflows');
    await p.screenshot({path:join(presetOutput,`repository-preset-${width}.png`),fullPage:true});
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.waitForURL(url=>!url.pathname.endsWith('/new'));
    const connections=await(await owner.request.get(`${origin}/api${root}/code-connections`)).json();
    const saved=connections.items.find(row=>row.name==='Preset Git '+width);
    assert(saved,'Preset connection must persist'); assert.equal(saved.provider,'git');assert.equal(String(saved.credential_id),'0');
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/code-connections/${saved.id}/edit`);
    await p.getByRole('combobox',{name:'代码平台',exact:true}).waitFor();
    assert.equal(await p.getByRole('button',{name:'纯 Git 拉取',exact:true}).count(),0);
    const deleted=await owner.request.delete(`${origin}/api${root}/code-connections/${saved.id}`,{headers});assert.equal(deleted.status(),204);
  }
  await p.setViewportSize({width:1280,height:800});
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
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/marketplace-sources`);
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.getByRole('textbox',{name:/^名称/}).fill('Legacy browser source');
    await chooseNative(p.getByLabel('仓库来源',{exact:true}), 'legacy');
    await chooseNative(p.getByLabel('外部仓库',{exact:true}), legacy.id);
    await p.getByRole('textbox',{name:/^API 接入点/}).fill(fixtureOrigin);
    await p.getByRole('textbox',{name:/^项目路径/}).fill('owner/repository');
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.waitForURL(`**/workspaces/${workspace_tenant}/marketplace-sources`);
    const originalSources=await(await owner.request.get(`${origin}/api${root}/marketplace-sources`)).json();const legacySource=originalSources.items.find(row=>row.name==='Legacy browser source');assert(legacySource);assert.equal(legacySource.repository_id,legacy.id);
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/marketplace-sources/${legacySource.id}/edit`);assert.equal(await p.getByLabel('外部仓库',{exact:true}).inputValue(),legacy.id);
    await p.getByRole('button',{name:'保存',exact:true}).click();await p.waitForURL(`**/workspaces/${workspace_tenant}/marketplace-sources`);
    const retained=await(await owner.request.get(`${origin}/api${root}/marketplace-sources/${legacySource.id}`)).json();assert.equal(retained.repository_id,legacy.id);assert.equal(new URL(retained.api_base).href,new URL(fixtureOrigin).href);assert.equal(retained.project,'owner/repository');
    await p.getByRole('button',{name:'创建',exact:true}).click();await p.getByRole('textbox',{name:/^名称/}).fill('Browser source');
    await chooseNative(p.getByLabel('仓库连接',{exact:true}), connection.id);await chooseNative(p.getByLabel('连接仓库',{exact:true}), fixtureOrigin+'/owner/repository.git');
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
    assert.equal(templates.total,4);assert.equal(templates.items.filter(row=>row.name==='Browser app').length,1);
    assert.equal(templates.items.filter(row=>JSON.parse(row.content).metadata?.name==='example-http').length,1);
    const example=templates.items.find(row=>JSON.parse(row.content).metadata?.name==='example-http');
    const nativeExample=templates.items.find(row=>JSON.parse(row.content).metadata?.name==='example-native-http');
    assert(nativeExample);
    for(const selected of [example,nativeExample]) for(const width of [1440,900,390,320]){
      const native=selected.id===nativeExample.id;
      await p.setViewportSize({width,height:900});
      await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/new?template=${selected.id}`);
      const generator=p.getByRole('region',{name:'从模板生成',exact:true});
      if(native){
        const pipelineParameter=JSON.parse(selected.parameters).find(parameter=>parameter.name==='pipeline');
        const pipelineInput=generator.getByRole('textbox',{name:pipelineParameter.label,exact:true});
        await pipelineInput.fill('sample-build');
      }
      await generator.getByRole('spinbutton',{name:'发布端口',exact:true}).fill('23180');
      const message=`HTTP scenario ${width}: 'quotes' $HOME $(false)`;
      await generator.getByRole('textbox',{name:'响应内容',exact:true}).fill(message);
      const rendered=p.waitForResponse(response=>response.url().endsWith(`/app-templates/${selected.id}/renderings`)&&response.request().method()==='POST');
      await generator.getByRole('button',{name:'生成配置',exact:true}).click();
      const response=await rendered;assert(response.ok(),await response.text());
      const manifest=JSON.parse((await response.json()).manifest);
      if(native){
        assert.equal(manifest.services[0].ports[0].container,23180);
        assert.equal(manifest.services[0].env[0].value,'23180');
        assert.equal(manifest.services[0].env[1].value,message);
        assert.equal(manifest.services[0].artifact.pipeline,'sample-build');
      }else{
        assert.equal(manifest.services[0].ports[0].published,23180);
        assert.equal(manifest.services[0].env[0].value,message);
        assert(!manifest.services[0].command.join(' ').includes(message));
      }
      await generator.getByRole('status').filter({hasText:'配置已填入'}).waitFor();
      assert(await generator.getByRole('button',{name:'生成配置',exact:true}).isEnabled());
      assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`page overflow at ${width}`);
      if(process.env.FERRY_SCENARIO_UI_OUTPUT){
        await mkdir(process.env.FERRY_SCENARIO_UI_OUTPUT,{recursive:true});
        await p.screenshot({path:join(process.env.FERRY_SCENARIO_UI_OUTPUT,`${native?'native':'http'}-template-${width}.png`)});
      }
    }
    const natsExample=templates.items.find(row=>JSON.parse(row.content).metadata?.name==='example-nats');
    assert(natsExample);
    for(const width of [1440,900,390,320]){
      await p.setViewportSize({width,height:900});
      await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/new?template=${natsExample.id}`);
      const generator=p.getByRole('region',{name:'从模板生成',exact:true});
      for(const parameter of JSON.parse(natsExample.parameters)){
        const label=new RegExp('^'+parameter.label.replace(/[.*+?^${}()|[\]\\]/g,'\\$&'));
        if(parameter.name==='token') await generator.getByRole('textbox',{name:label}).fill('nats-client-secret');
        if(parameter.name.startsWith('advertise_')) await generator.getByRole('textbox',{name:label}).fill('client.example.invalid:'+({a:4222,b:4223,c:4224}[parameter.name.at(-1)]));
      }
      const rendered=p.waitForResponse(response=>response.url().endsWith(`/app-templates/${natsExample.id}/renderings`)&&response.request().method()==='POST');
      await generator.getByRole('button',{name:'生成配置',exact:true}).click();
      const response=await rendered;assert(response.ok(),await response.text());
      const manifest=JSON.parse((await response.json()).manifest);
      assert.equal(manifest.services.length,3);
      assert.equal(new Set(manifest.services.map(service=>service.volumes[0].name)).size,3);
      assert(manifest.services.every(service=>service.env[2].valueFrom==='variable/nats-client-secret'));
      assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`NATS page overflow at ${width}`);
      if(process.env.FERRY_SCENARIO_UI_OUTPUT){
        await mkdir(process.env.FERRY_SCENARIO_UI_OUTPUT,{recursive:true});
        await p.screenshot({path:join(process.env.FERRY_SCENARIO_UI_OUTPUT,`nats-template-${width}.png`)});
      }
    }
    const deployments=await(await owner.request.get(`${origin}/api${root}/deployments`)).json();
    assert.equal(deployments.total,0,'Selecting and rendering an example must not deploy or save an application');
    await p.setViewportSize({width:1280,height:800});
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/marketplace-sources`);
    await p.getByRole('row').filter({has:p.getByRole('cell',{name:'Browser source',exact:true})}).getByRole('button',{name:'删除',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除',exact:true}).click();
    await p.getByRole('cell',{name:'Browser source',exact:true}).waitFor({state:'detached'});
    assert(authorizedSyncs>0,'marketplace sync used the selected connection credential');
    assert.deepEqual(errors,[]);
    console.log('PASS: legacy source create/edit retention, named connection repository source creation, authenticated HTTP sync, catalog detail, independent template import, container, Native and NATS scenario parameters at four widths without deployment and source deletion');
  } finally {fixture.closeAllConnections();await new Promise(r=>fixture.close(r));}
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
