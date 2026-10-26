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
  const host=await post(owner,root+'/hosts',{name:'Template host',connection:'runner'});
  await post(owner,root+'/variables',{name:'token',scope:'deploy',kind:'secret',value:'secret-not-in-template'});
  const get=async path => {const r=await owner.request.get(`${origin}/api${path}`); assert(r.ok()); return r.json();};
  for (const [group,resource,target,field,label] of [
    ['deploy','app-templates','apps','manifest','应用 manifest（YAML / JSON）'],
    ['build','pipeline-templates','pipelines','definition','定义（JSON）'],
  ]) {
    const app=field==='manifest';
    const name=`UI ${resource}`;
    await p.goto(`${origin}/tenant/${group}/${resource}`);
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
    await p.waitForFunction(text=>!document.body.innerText.includes(text),name+' edited');
    row=p.getByRole('row').filter({hasText:name});
    await row.getByRole('link',{name:'使用模板',exact:true}).click();
    await p.getByText(/版本 1/).waitFor();
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
    const generated=JSON.parse(await p.getByLabel(label,{exact:false}).inputValue());
    if(app) assert.equal(generated.services[0].env[0].valueFrom,'variable/token');
    else { assert.equal(generated.steps[0].run,'echo build false'); assert.equal(generated.steps[0].timeout_seconds,30); }
    assert(!JSON.stringify(generated).includes('secret-not-in-template'));
    await p.getByLabel(app?'镜像':'构建命令',{exact:false}).fill(app?'nginx:latest':'echo revised');
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
    await p.getByLabel('名称',{exact:false}).fill('Generated '+resource);
    await p.getByLabel('目标主机',{exact:false}).click();
    await p.getByRole('option',{name:'Template host',exact:true}).click();
    await create.click();
    await p.getByRole('cell',{name:'Generated '+resource,exact:true}).waitFor();
    const objects=await get(root+'/'+(app?'deployments':'pipelines'));
    assert.equal(objects.total,1);
    assert.equal(objects.items[0].target_ref,host.id);
    await p.goto(`${origin}/tenant/${group}/${target}/new`);
    await p.getByRole('region',{name:'从模板生成'}).getByRole('combobox').click();
    await p.getByRole('option',{name,exact:true}).click();
    await p.getByText(/版本 2/).waitFor();
    assert(await p.getByRole('button',{name:'创建',exact:true}).isDisabled());
    console.log(`PASS ${resource}: UI create/copy/edit/delete, template selection, parameters, generate, conflict/reload, real resource creation`);
  }
  assert.deepEqual(errors,[]);
  console.log('PASS no browser errors');
} finally {
  if (browser) await browser.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  await log.close();
  await rm(work, { recursive: true, force: true });
}
