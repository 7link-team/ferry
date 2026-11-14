import { startBlank } from './ui-template-chooser.mjs';
/** Real Chromium acceptance for template management and generated resources. */
import assert from 'node:assert/strict';
import { parseDocument } from 'yaml';
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
  const context = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, viewport: { width: 1280, height: 800 } });
  const errors = [];

  const headers = { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' };
  const post = async (ctx, path, data) => {
    const response = await ctx.request.post(`${origin}/api${path}`, { headers, data });
    assert(response.ok(), `${path}: ${await response.text()}`); return response.json();
  };
  await post(context, '/setup', {email:'admin@example.invalid',name:'Admin',password:'temporary-admin-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const owner = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, permissions: ['clipboard-read','clipboard-write'] });
  await post(owner, '/users', {email:'owner@example.invalid',name:'Owner',password:'temporary-owner-password'});
  const workspace_tenant = (await post(owner, '/workspaces', {name:'Tenant',slug:'tenant'})).id;
  const p = await owner.newPage();
  const readWorkflow = async () => {
    await p.getByRole('tab',{name:'高级 YAML',exact:true}).click();
    await p.locator('.monaco-editor .view-lines').click();
    await p.keyboard.press('ControlOrMeta+A');
    await p.keyboard.press('ControlOrMeta+C');
    return parseDocument(await p.evaluate(() => navigator.clipboard.readText())).toJS();
  };
  p.on('pageerror', e => errors.push(e.message));
  p.on('dialog', d => d.accept());
  const root=`/workspaces/${workspace_tenant}`;
  const host=await post(owner,root+'/nodes',{name:'Template host',connection:'runner'});
  await post(owner,root+'/variables',{name:'token',scope:'deploy',kind:'secret',value:'secret-not-in-template'});
  const get=async path => {const r=await owner.request.get(`${origin}/api${path}`); assert(r.ok()); return r.json();};
  for (const [group,resource,target,field,label] of [
    ['deploy','app-templates','apps','manifest','应用配置'],
    ['build','pipeline-templates','pipelines','definition','GitHub Actions YAML'],
  ]) {
    const app=field==='manifest';
    const name=`UI ${resource}`;
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/${resource}`);
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.locator('[data-auto-field="name"] input').fill(name);
    if (app) {
      await p.getByRole('button',{name:'高级 JSON',exact:true}).click(); await writeEditor(p,p.getByRole('textbox',{name:'高级 JSON',exact:true}),JSON.stringify([{name:'image',label:'镜像',type:'string',default:'nginx:alpine'},{name:'token',label:'令牌',type:'string',secret:true}]));
      await p.getByRole('button',{name:'源码编辑',exact:true}).click();
      await writeEditor(p,p.getByRole('textbox',{name:label,exact:true}),JSON.stringify({apiVersion:'ferry/v1',kind:'Application',metadata:{name:'web'},services:[{name:'web',image:'{{ .Values.image }}',env:[{name:'TOKEN',valueFrom:'{{ .Values.token }}'}]}]}));
    }
    if (!app) {
      assert.equal(await p.getByLabel('参数定义（JSON 数组）',{exact:true}).count(),0,'new pipeline templates no longer expose dynamic parameters');
      // Seed a legacy record through the API; the UI must preserve compatibility
      // while new templates use complete Actions YAML without dynamic parameters.
      await post(owner,root+'/'+resource,{
        name,parameters:JSON.stringify([{name:'command',label:'构建命令',type:'string',default:'echo build'},{name:'flag',label:'布尔参数',type:'boolean',default:false},{name:'timeout',label:'超时',type:'integer',min:1,max:60}]),
        content:JSON.stringify({engine:'github-actions',workflow:'name: Template\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    defaults:\n      run:\n        working-directory: /tmp/build/${{ github.run_number }}\n    steps:\n      - name: test\n        run: "{{ .Values.command }} {{ .Values.flag }}"\n        timeout-minutes: 1\n        env:\n          TIMEOUT: "{{ .Values.timeout }}"\n',executor_bindings:[],checkout_bindings:[],graph_layout:{}}),
      });
      await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/${resource}`);
    }
    if (app) await p.getByRole('button',{name:'创建',exact:true}).click();
    let row=p.getByRole('row').filter({hasText:name});
    await row.getByRole('link',{name:'复制',exact:true}).click();
    await p.locator('[data-auto-field="name"] input').fill(name+' copy');
    await p.getByRole('button',{name:'创建',exact:true}).click();
    await p.getByRole('row').filter({hasText:name+' copy'}).getByRole('link',{name:'编辑',exact:true}).click();
    await p.locator('[data-auto-field="name"] input').fill(name+' edited');
    await p.getByRole('button',{name:'保存',exact:true}).click();
    await p.getByRole('row').filter({hasText:name+' edited'}).getByRole('button',{name:'删除模板',exact:true}).click();
    await p.getByRole('dialog').getByRole('button',{name:'确认删除模板',exact:true}).click();
    await p.waitForFunction(text=>!document.body.innerText.includes(text),name+' edited');
    row=p.getByRole('row').filter({hasText:name});
    await row.getByRole('link',{name:'使用模板',exact:true}).click();
    await p.getByRole('heading',{level:3,name:/版本 1/}).waitFor();
    if (!app) {
      const dialog=p.getByRole('dialog');
      // The modal chooser now splits template list and parameters into two steps; a ?template= link opens step 2.
      await dialog.getByRole('list',{name:'创建步骤'}).getByRole('listitem').filter({hasText:'模板列表'}).waitFor();
      assert.equal(await dialog.locator('li[aria-current="step"]').innerText(),'2. 模板参数');
      assert.equal(await dialog.getByRole('combobox').count(),1,'only the boolean parameter uses a select');
      await dialog.getByLabel('超时',{exact:true}).fill('');
      assert(await dialog.getByRole('button',{name:'使用此模板',exact:true}).isDisabled(),'missing required timeout blocks rendering');
      await dialog.getByLabel('超时',{exact:true}).fill('30');
      const template=(await get(root+'/'+resource)).items.find(item=>item.name===name);
      const edited=await owner.request.patch(`${origin}/api${root}/${resource}/${template.id}`,{headers,data:{...template,description:'Changed concurrently'}});
      assert(edited.ok());
      await dialog.getByRole('button',{name:'使用此模板',exact:true}).click();
      await dialog.getByRole('alert').filter({hasText:'模板已变化'}).waitFor();
      await dialog.getByRole('button',{name:'重新加载模版',exact:true}).click();
      await dialog.getByRole('heading',{level:3,name:/版本 2/}).waitFor();
      await dialog.getByLabel('超时',{exact:true}).fill('30');
      await dialog.getByRole('button',{name:'使用此模板',exact:true}).click();
      await dialog.waitFor({state:'hidden'});
      const generated=await readWorkflow();
      const step=generated.jobs.build.steps.find(step=>step.name==='test');
      assert.equal(step.run,'echo build false');
      assert.equal(step['timeout-minutes'],1);
      assert.equal(step.env.TIMEOUT,30);
      assert.equal(generated.jobs.build.defaults.run['working-directory'],'/tmp/build/${{ github.run_number }}');
      assert(!JSON.stringify(generated).includes('secret-not-in-template'));
      await p.locator('[data-auto-field="name"] input').fill('Generated '+resource);
      await p.getByRole('alert').getByRole('button',{name:'build',exact:true}).click();
      await p.getByLabel('任务执行器',{exact:true}).selectOption(host.id);
      await p.getByRole('button',{name:'保存',exact:true}).click();
      await p.getByRole('cell',{name:'Generated '+resource,exact:true}).waitFor();
      const objects=await get(root+'/pipelines');
      assert.equal(objects.total,1);
      assert(objects.items[0].executor_bindings.some(binding=>binding.job_id==='build' && binding.target_ref===host.id));
      await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/pipelines/new`);
      await startBlank(p);
      await p.getByRole('button',{name:'从模板导入',exact:true}).click();
      await p.getByRole('dialog').getByRole('radio',{name:new RegExp(name)}).click();
      await p.getByRole('dialog').getByRole('button',{name:'下一步：填写参数',exact:true}).click();
      await p.getByRole('dialog').getByRole('heading',{level:3,name:/版本 2/}).waitFor();
      assert(await p.getByRole('button',{name:'保存',exact:true,includeHidden:true}).isDisabled(),'pending template generation blocks saving');
      console.log('PASS pipeline-templates: CRUD confirmation, modal template list, typed parameters, stale conflict/reload, generated values and real pipeline creation');
      continue;
    }
    // Application creation shows the chooser as a page section (dialog mode is kept for other resources).
    const dialog=chooser(p);
    const create=p.getByRole('button',{name:'创建',exact:true,includeHidden:true});
    await dialog.waitFor();
    await assertCreationBlocked(create,'chooser blocks background creation');
    await dialog.getByLabel('令牌（密钥变量名）',{exact:true}).fill('token');
    const template=(await get(root+'/'+resource)).items.find(item=>item.name===name);
    const edited=await owner.request.patch(`${origin}/api${root}/${resource}/${template.id}`,{headers,data:{...template,description:'Changed concurrently'}});
    assert(edited.ok());
    await dialog.getByRole('button',{name:'使用此模板',exact:true}).click();
    await dialog.getByRole('alert').filter({hasText:'模板已变化'}).waitFor();
    await assertCreationBlocked(create,'stale version blocks background creation');
    await dialog.getByRole('button',{name:'重新加载模版',exact:true}).click();
    await dialog.getByRole('heading',{level:3,name:/版本 2/}).waitFor();
    await dialog.getByLabel('令牌（密钥变量名）',{exact:true}).fill('token');
    await dialog.getByLabel('镜像',{exact:true}).fill('nginx:latest');
    await dialog.getByRole('button',{name:'使用此模板',exact:true}).click();
    await dialog.waitFor({state:'hidden'});
    await p.getByRole('button',{name:'源码编辑',exact:true}).click();
    const generated=parseDocument(await readEditor(p,p.getByRole('textbox',{name:label,exact:true}))).toJS();
    assert.equal(generated.services[0].env[0].valueFrom,'variable/token');
    assert.equal(generated.services[0].image,'nginx:latest');
    assert(!JSON.stringify(generated).includes('secret-not-in-template'));
    await p.getByRole('button',{name:'表单编辑',exact:true}).click();
    await p.locator('[data-auto-field="name"] input').fill('Generated '+resource);
    await p.getByRole('combobox',{name:'目标主机',exact:true}).click();
    await p.getByRole('option',{name:'Template host',exact:true}).click();
    await create.click();
    // Application creation now lands on the new deployment's detail page instead of the list.
    await p.getByRole('heading',{level:2,name:'Generated '+resource,exact:true}).waitFor();
    const objects=await get(root+'/'+(app?'deployments':'pipelines'));
    assert.equal(objects.total,1);
    assert.equal(objects.items[0].target_ref,host.id);
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/${target === 'apps' ? 'deployments' : target}/new`);
    await p.getByRole('radio',{name:new RegExp(name)}).click();
    await chooser(p).getByRole('button',{name:'下一步：填写参数',exact:true}).click();
    await p.getByRole('heading',{level:3,name:/版本 2/}).waitFor();
    await assertCreationBlocked(p.getByRole('button',{name:'创建',exact:true,includeHidden:true}),'pending template generation blocks creation');
    console.log(`PASS ${resource}: UI create/copy/edit/delete, template selection, parameters, generate, conflict/reload, real resource creation`);
  }
  // Pagination and layout remain a permanent CI contract, including a narrow phone.
  for(let index=0;index<13;index++)await post(owner,root+'/app-templates',{
    name:`Paged chooser ${String(index).padStart(2,'0')}`,description:'CI pagination',
    parameters:JSON.stringify([{name:'image',label:'分页镜像',type:'string',default:'nginx:alpine'}]),
    content:JSON.stringify({apiVersion:'ferry/v1',kind:'Application',metadata:{name:'paged'},services:[{name:'web',image:'{{ .Values.image }}'}]})
  });
  for(const width of [1440,900,390,320]){
    await p.setViewportSize({width,height:960});
    await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/deployments/new`);
    const dialog=chooser(p);
    await dialog.getByRole('textbox',{name:'搜索',exact:true}).fill('Paged chooser');
    await p.waitForFunction(()=>[...document.querySelectorAll('[role="dialog"] input[type="radio"], section[data-template-selection] input[type="radio"]')].filter(el=>el.closest('label')?.textContent.includes('Paged chooser')).length===12);
    const heading=await dialog.getByRole('heading').first().boundingBox(),footer=await dialog.locator('.dialog-footer').boundingBox();
    await dialog.locator('[data-template-catalog]').evaluate(el=>el.scrollTop=el.scrollHeight);
    assert(Math.abs(heading.y-(await dialog.getByRole('heading').first().boundingBox()).y)<1,'dialog title is fixed');
    assert(Math.abs(footer.y-(await dialog.locator('.dialog-footer').boundingBox()).y)<1,'dialog actions are fixed');
    await dialog.getByRole('button',{name:'下一页',exact:true}).click();
    await p.waitForFunction(()=>[...document.querySelectorAll('[role="dialog"] input[type="radio"], section[data-template-selection] input[type="radio"]')].filter(el=>el.closest('label')?.textContent.includes('Paged chooser')).length===1);
    await dialog.getByRole('radio',{name:/Paged chooser/}).check();
    // Parameters are a separate chooser step: fill them, return to the list, page back, and the
    // selection plus entered parameters must survive.
    await dialog.getByRole('button',{name:'下一步：填写参数',exact:true}).click();
    await dialog.getByLabel('分页镜像',{exact:true}).fill('nginx:stable');
    await dialog.getByRole('button',{name:'更换模板',exact:true}).click();
    await dialog.getByRole('button',{name:'上一页',exact:true}).click();
    await p.waitForFunction(()=>[...document.querySelectorAll('[role="dialog"] input[type="radio"], section[data-template-selection] input[type="radio"]')].filter(el=>el.closest('label')?.textContent.includes('Paged chooser')).length===12);
    await dialog.getByRole('button',{name:'下一步：填写参数',exact:true}).click();
    assert.equal(await dialog.getByLabel('分页镜像',{exact:true}).inputValue(),'nginx:stable');
    await dialog.getByRole('button',{name:'使用此模板',exact:true}).click();
    await dialog.waitFor({state:'hidden'});
    assert.equal(await p.locator('[data-form-section]').count(),3);
    // Section navigation is now 基本信息 / 部署目标 / 应用定义 (services live inside 应用定义).
    await p.getByRole('navigation',{name:'配置分类'}).getByRole('link',{name:'应用定义',exact:true}).click();
    await p.waitForFunction(()=>document.querySelector('[data-section-navigation] [aria-current]')?.textContent==='应用定义');
    assert(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`no full-page overflow at ${width}`);
    assert(await p.locator('input, textarea').evaluateAll(nodes=>nodes.some(node=>node.value==='nginx:stable')));
  }
  console.log('PASS Office chooser: server pagination/search, fixed header/actions, retained parameters, continuous form and section highlighting at four widths');
  await p.goto(`${origin}/zh-CN/workspaces/${workspace_tenant}/config-templates`);
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByLabel('名称',{exact:true}).fill('Runtime file');
  await writeEditor(p,p.getByRole('textbox',{name:'应用配置',exact:true}),'port={{ .Values.port }}\n');
  await p.getByRole('button',{name:'创建',exact:true}).click();
  await p.getByRole('row').filter({hasText:'Runtime file'}).getByRole('link',{name:'编辑',exact:true}).click();
  await writeEditor(p,p.getByRole('textbox',{name:'应用配置',exact:true}),'listen={{ .Values.port }}\n');
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

/** Template chooser: a modal dialog for resource forms, a page section for application creation. */
function chooser(page){return page.getByRole('dialog').or(page.locator('section[data-template-selection]')).first();}
/** In dialog mode the background create button stays disabled; in page mode it is not rendered at all. */
async function assertCreationBlocked(create,message){if(await create.count())assert(await create.isDisabled(),message);}
async function focusEditor(locator){await locator.locator('xpath=ancestor::div[contains(@class,"monaco-editor")][1]').locator('.view-lines').click();}
async function readEditor(page,locator){await focusEditor(locator);await page.keyboard.press('ControlOrMeta+A');await page.keyboard.press('ControlOrMeta+C');return page.evaluate(()=>navigator.clipboard.readText());}
async function writeEditor(page,locator,value){await focusEditor(locator);await page.keyboard.press('ControlOrMeta+A');await page.evaluate(value=>navigator.clipboard.writeText(value),value);await page.keyboard.press('ControlOrMeta+V');}
