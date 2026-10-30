/** Real Chromium acceptance for template management and generated resources. */
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
  const host=await post(owner,root+'/nodes',{name:'Template host',connection:'runner'});
  await post(owner,root+'/variables',{name:'token',scope:'deploy',kind:'secret',value:'secret-not-in-template'});
  const get=async path => {const r=await owner.request.get(`${origin}/api${path}`); assert(r.ok()); return r.json();};
  for (const [group,resource,target,field,label] of [
    ['deploy','app-templates','apps','manifest','应用配置源码（YAML / JSON）'],
    ['build','pipeline-templates','pipelines','definition','定义（JSON）'],
  ]) {
    const app=field==='manifest';
    const name=`UI ${resource}`;
    await p.goto(`${origin}/workspaces/${workspace_tenant}/${resource}`);
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.getByLabel('名称',{exact:false}).fill(name);
    if (app) {
      await p.getByLabel('参数定义（JSON 数组）',{exact:false}).fill(JSON.stringify([{name:'image',label:'镜像',type:'string',default:'nginx:alpine'},{name:'token',label:'令牌',type:'string',secret:true}]));
      await p.getByLabel('应用模板（YAML / JSON）',{exact:false}).fill(JSON.stringify({apiVersion:'ferry/v1',kind:'Application',metadata:{name:'web'},services:[{name:'web',image:'{{ .Values.image }}',env:[{name:'TOKEN',valueFrom:'{{ .Values.token }}'}]}]}));
    }
    if (!app) {
      await p.getByLabel('参数定义（JSON 数组）',{exact:false}).fill(JSON.stringify([{name:'command',label:'构建命令',type:'string',default:'echo build'},{name:'flag',label:'布尔参数',type:'boolean',default:false},{name:'timeout',label:'超时',type:'integer',min:1,max:60}]));
      await p.getByLabel('流水线模板（JSON / YAML）',{exact:false}).fill(JSON.stringify({workdir:'/tmp/build/{{run}}',steps:[{name:'test',run:'{{ .Values.command }} {{ .Values.flag }}',timeout_seconds:'{{ .Values.timeout }}'}]}));
    }
    await p.getByRole('button',{name:'创建',exact:true}).click();
    let row=p.getByRole('row').filter({hasText:name});
    await row.getByRole('link',{name:'复制',exact:true}).click();
    await p.getByLabel('名称',{exact:false}).fill(name+' copy');
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.getByRole('row').filter({hasText:name+' copy'}).getByRole('link',{name:'编辑',exact:true}).click();
    await p.getByLabel('名称',{exact:false}).fill(name+' edited');
    await p.getByRole('button',{name:'保存',exact:true}).click();
    await p.getByRole('row').filter({hasText:name+' edited'}).getByRole('button',{name:'删除模板',exact:true}).click();
    await p.getByRole('dialog').getByRole('button',{name:'确认删除模板',exact:true}).click();
    await p.waitForFunction(text=>!document.body.innerText.includes(text),name+' edited');
    row=p.getByRole('row').filter({hasText:name});
    await row.getByRole('link',{name:'使用模板',exact:true}).click();
    await p.getByText(/版本 1/).waitFor();
    if (!app) {
      const dialog=p.getByRole('dialog');
      await dialog.getByRole('listbox',{name:'模板列表'}).waitFor();
      assert.equal(await dialog.getByRole('combobox').count(),1,'only the boolean parameter uses a select');
      await dialog.getByLabel('超时',{exact:false}).fill('');
      await dialog.getByRole('button',{name:'生成配置',exact:true}).click();
      await dialog.getByRole('alert').filter({hasText:'请填写参数 timeout'}).waitFor();
      await dialog.getByLabel('超时',{exact:false}).fill('30');
      const template=(await get(root+'/'+resource)).items.find(item=>item.name===name);
      const edited=await owner.request.patch(`${origin}/api${root}/${resource}/${template.id}`,{headers,data:{...template,description:'Changed concurrently'}});
      assert(edited.ok());
      await dialog.getByRole('button',{name:'生成配置',exact:true}).click();
      await dialog.getByRole('alert').filter({hasText:'模板已变化'}).waitFor();
      await dialog.getByRole('button',{name:'重新载入模板',exact:true}).click();
      await dialog.getByText(/版本 2/).waitFor();
      await dialog.getByLabel('超时',{exact:false}).fill('30');
      await dialog.getByRole('button',{name:'生成配置',exact:true}).click();
      await dialog.waitFor({state:'hidden'});
      const generated=JSON.parse(await p.getByLabel(label,{exact:false}).inputValue());
      assert.equal(generated.steps[0].run,'echo build false');
      assert.equal(generated.steps[0].timeout_seconds,30);
      assert(!JSON.stringify(generated).includes('secret-not-in-template'));
      await p.getByRole('textbox',{name:'流水线名称',exact:true}).fill('Generated '+resource);
      await p.getByRole('button',{name:'流水线设置',exact:true}).click();
      await p.getByLabel('目标主机',{exact:false}).click();
      await p.getByRole('option',{name:'Template host',exact:true}).click();
      await p.getByRole('button',{name:'保存',exact:true}).click();
      await p.getByRole('cell',{name:'Generated '+resource,exact:true}).waitFor();
      const objects=await get(root+'/pipelines');
      assert.equal(objects.total,1);
      assert.equal(objects.items[0].target_ref,host.id);
      await p.goto(`${origin}/workspaces/${workspace_tenant}/pipelines/new`);
      await p.getByRole('button',{name:'从模板导入',exact:true}).click();
      await p.getByRole('dialog').getByRole('option',{name,exact:true}).click();
      await p.getByRole('dialog').getByText(/版本 2/).waitFor();
      assert(await p.getByRole('button',{name:'保存',exact:true,includeHidden:true}).isDisabled(),'pending template generation blocks saving');
      console.log('PASS pipeline-templates: CRUD confirmation, modal template list, typed parameters, stale conflict/reload, generated values and real pipeline creation');
      continue;
    }
    const create=p.getByRole('button',{name:'创建',exact:true});
    assert(await create.isDisabled(),'must generate first');
    if(app) await p.getByLabel('令牌（密钥变量名）',{exact:false}).fill('token');
    else {
      await p.getByLabel('超时',{exact:false}).fill('');
      await p.getByRole('button',{name:'生成配置',exact:true}).click();
      await p.getByRole('alert').filter({hasText:'请填写参数 timeout'}).waitFor();
      await p.getByLabel('超时',{exact:false}).fill('30');
    }
    await p.getByRole('button',{name:'生成配置',exact:true}).click();
    await p.getByText('配置已填入下方编辑器',{exact:false}).waitFor();
    if(app) await p.getByRole('button',{name:'源码编辑',exact:true}).click();
    const generated=JSON.parse(await p.getByLabel(label,{exact:false}).inputValue());
    if(app) assert.equal(generated.services[0].env[0].valueFrom,'variable/token');
    else { assert.equal(generated.steps[0].run,'echo build false'); assert.equal(generated.steps[0].timeout_seconds,30); }
    assert(!JSON.stringify(generated).includes('secret-not-in-template'));
    await p.getByRole('region',{name:'从模板生成'}).getByLabel(app?'镜像':'构建命令',{exact:false}).fill(app?'nginx:latest':'echo revised');
    // The field invalidates local generation first; its effect reports the
    // invalid state to the parent form on the next React render.
    await p.waitForFunction(() => [...document.querySelectorAll('button')].find(button => button.textContent === '创建')?.disabled, null, {timeout:5000});
    assert(await create.isDisabled(),'changed parameters invalidate generation');
    const template=(await get(root+'/'+resource)).items[0];
    const edited=await owner.request.patch(`${origin}/api${root}/${resource}/${template.id}`,{headers,data:{...template,description:'Changed concurrently'}});
    assert(edited.ok());
    await p.getByRole('button',{name:'生成配置',exact:true}).click();
    await p.getByRole('alert').filter({hasText:'模板已变化'}).waitFor();
    assert(await create.isDisabled(),'stale version blocks creation');
    await p.getByRole('button',{name:'重新载入模板',exact:true}).click();
    await p.getByText(/版本 2/).waitFor();
    if(app) await p.getByLabel('令牌（密钥变量名）',{exact:false}).fill('token');
    else {
      await p.getByLabel('超时',{exact:false}).fill('');
      await p.getByRole('button',{name:'生成配置',exact:true}).click();
      await p.getByRole('alert').filter({hasText:'请填写参数 timeout'}).waitFor();
      await p.getByLabel('超时',{exact:false}).fill('30');
    }
    await p.getByRole('button',{name:'生成配置',exact:true}).click();
    await p.getByText('配置已填入下方编辑器',{exact:false}).waitFor();
    if(app) await p.getByRole('button',{name:'表单编辑',exact:true}).click();
    await p.getByRole('textbox',{name:/^名称/}).fill('Generated '+resource);
    await p.getByLabel('目标主机',{exact:false}).click();
    await p.getByRole('option',{name:'Template host',exact:true}).click();
    await create.click();
    await p.getByRole('cell',{name:'Generated '+resource,exact:true}).waitFor();
    const objects=await get(root+'/'+(app?'deployments':'pipelines'));
    assert.equal(objects.total,1);
    assert.equal(objects.items[0].target_ref,host.id);
    await p.goto(`${origin}/workspaces/${workspace_tenant}/${target === 'apps' ? 'deployments' : target}/new`);
    await p.getByRole('region',{name:'从模板生成'}).getByRole('combobox').click();
    await p.getByRole('option',{name,exact:true}).click();
    await p.getByText(/版本 2/).waitFor();
    assert(await p.getByRole('button',{name:'创建',exact:true}).isDisabled());
    console.log(`PASS ${resource}: UI create/copy/edit/delete, template selection, parameters, generate, conflict/reload, real resource creation`);
  }
  await p.goto(`${origin}/workspaces/${workspace_tenant}/config-templates`);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByLabel('名称',{exact:false}).fill('Runtime file');
  await p.getByLabel('应用配置文件内容',{exact:false}).fill('port={{ .Values.port }}\n');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'Runtime file'}).getByRole('link',{name:'编辑',exact:true}).click();
  await p.getByLabel('应用配置文件内容',{exact:false}).fill('listen={{ .Values.port }}\n');
  await p.getByRole('button',{name:'保存',exact:true}).click();
  const configs=await get(root+'/config-templates');
  assert.equal(configs.items[0].content,'listen={{ .Values.port }}\n');
  await p.getByRole('row').filter({hasText:'Runtime file'}).getByRole('button',{name:'删除模板',exact:true}).click();
  await p.getByRole('dialog').getByRole('button',{name:'确认删除模板',exact:true}).click();
  await p.waitForFunction(()=>!document.body.innerText.includes('Runtime file'));
  console.log('PASS configuration template UI create/edit/delete with plain text content');
  assert.deepEqual(errors,[]);
  console.log('PASS no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
