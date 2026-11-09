/** Focused real ReactFlow browser interactions, with no Rust/Go build required. */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdir } from 'node:fs/promises';
import { parseDocument, stringify } from 'yaml';
import { build } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { createServer as createHttpServer } from 'node:http';
import { chromium } from 'playwright';
const entryId = fileURLToPath(new URL('./__canvas-entry.tsx', import.meta.url));
// Build the real component fixture without app-wide development scanning/HMR.
const bundle = await build({
 configFile: false,
 logLevel: 'warn',
 resolve: { alias: { '@': fileURLToPath(new URL('../src', import.meta.url)) } },
 plugins: [react(), tailwindcss(), { name: 'canvas-test',
  resolveId(id) { if(id === entryId) return id; },
  load(id) { if(id === entryId) return `import React, {useState} from 'react';import {createRoot} from 'react-dom/client';import {ActionsEditor} from '/src/features/pipelines/ActionsEditor.tsx';import {DEFAULT_ACTIONS,editActionStep} from '/src/features/pipelines/actions-workflow.ts';import '/src/style.css';function App(){const [draft,setDraft]=useState(JSON.parse(localStorage.getItem('canvas-draft')||'null')||{definition:DEFAULT_ACTIONS});window.canvasDraft=draft;window.setCanvasDraft=setDraft;window.canvasSubmits??=0;return <main style={{padding:16,maxWidth:'100%',height:'100vh',display:'flex',flexDirection:'column'}}><form style={{display:'flex',flexDirection:'column',minHeight:0,flex:1}} onSubmit={event=>{event.preventDefault();window.canvasSubmits++;}}><ActionsEditor draft={draft} options={[{value:'host:local',label:'Local host'},{value:'host-group:group',label:'Local group'}]} readOnly={false} fillHeight onChange={patch=>setDraft(d=>{const next={...d,...patch};localStorage.setItem('canvas-draft',JSON.stringify(next));return next})} onValidityChange={invalid=>{window.canvasInvalid=invalid;}} renderCheckout={(job,index)=> <button type="button" onClick={()=>setDraft(d=>({...d,definition:editActionStep(d.definition,job,index,{id:'source'}),checkout_bindings:[{job_id:job,step_id:'source',repository_id:'repo'}]}))}>Repository selector</button>}/></form></main>};createRoot(document.getElementById('root')).render(<App/>);`; }
 }],
 build: { write: false, rollupOptions: { input: entryId } },
});
const output = bundle.output;
const scripts = output.filter(item => item.type === 'chunk' && item.isEntry);
const styles = output.filter(item => item.fileName.endsWith('.css'));
const server = createHttpServer((request, response) => {
 const path = new URL(request.url, 'http://localhost').pathname.slice(1);
 const asset = output.find(item => item.fileName === path);
 if (asset) {
  response.setHeader('Content-Type', asset.type === 'chunk' || path.endsWith('.js') ? 'text/javascript' : path.endsWith('.css') ? 'text/css' : 'application/octet-stream');
  response.end(asset.type === 'chunk' ? asset.code : asset.source);
  return;
 }
 response.setHeader('Content-Type', 'text/html');
 response.end(`<html><head><meta name="viewport" content="width=device-width,initial-scale=1"/>${styles.map(item => `<link rel="stylesheet" href="/${item.fileName}">`).join('')}</head><body><div id="root"></div>${scripts.map(item => `<script type="module" src="/${item.fileName}"></script>`).join('')}</body></html>`);
});
let browser;
const screenshots='data/maintenance/actions-canvas';
await mkdir(screenshots,{recursive:true});
try {
 const chrome=process.env.PLAYWRIGHT_CHROME_PATH||'C:/Program Files/Google/Chrome/Application/chrome.exe';
 await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));browser=await chromium.launch({headless:true,...(existsSync(chrome)?{executablePath:chrome}:{})});
 const context=await browser.newContext({locale:'zh-CN',extraHTTPHeaders:{'Accept-Language':'zh-CN'},viewport:{width:1280,height:900},permissions:['clipboard-read','clipboard-write']});
 const page=await context.newPage(),errors=[],workers=[],failed=[];page.on('pageerror',error=>errors.push(error.message));page.on('response',response=>{if(response.status()>=400)failed.push(`${response.status()} ${response.url()}`)});page.on('worker',worker=>workers.push(worker.url()));await page.goto(`http://127.0.0.1:${server.address().port}/__canvas`);
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 await page.locator('[aria-label="流水线画布"]').waitFor();
 const bottomGap=await page.evaluate(()=>{const main=document.querySelector('main').getBoundingClientRect(),canvas=document.querySelector('[aria-label="流水线画布"]').getBoundingClientRect();return main.bottom-canvas.bottom});
 assert(bottomGap<32,`canvas leaves excessive blank space: ${bottomGap}px`);
 const initialDraft=await page.evaluate(()=>window.canvasDraft);
 const formFixture={definition:'# keep form comment\n'+stringify({name:'Form fixture',on:'workflow_dispatch',jobs:{build:{'runs-on':'self-hosted','x-unknown':{keep:true},steps:[{name:'Ordinary form',run:'echo before',env:{MODE:'old'},'x-unknown':{keep:true}}]}}})};
 await page.evaluate(draft=>window.setCanvasDraft(draft),formFixture);
 await page.locator('.react-flow__node[data-id="build"]').getByRole('button',{name:/Ordinary form/}).click();
 const ordinaryPanel=page.getByRole('complementary',{name:'步骤配置',exact:true});
 const idHelp=ordinaryPanel.getByRole('button',{name:'步骤 ID（用于引用输出） · 用法帮助',exact:true});
 await idHelp.hover();
 await page.getByRole('tooltip').filter({hasText:'同一任务内唯一'}).waitFor();
 await idHelp.focus();await idHelp.press('Escape');
 assert.equal(await page.getByRole('tooltip').count(),0);
 assert(await ordinaryPanel.getByRole('region',{name:'环境变量',exact:true}).evaluate(env=>env.getBoundingClientRect().top<env.closest('aside').querySelector('textarea').getBoundingClientRect().top));
 assert.equal(await ordinaryPanel.locator('.monaco-editor:visible').count(),0);
 await ordinaryPanel.getByRole('textbox',{name:'执行命令',exact:true}).fill('echo from-form');
 await ordinaryPanel.getByLabel('工作目录',{exact:true}).fill('./app');
 await ordinaryPanel.getByLabel('运行条件（if）',{exact:true}).selectOption('success()');
 await ordinaryPanel.getByLabel('环境变量.MODE',{exact:true}).fill('production');
 await ordinaryPanel.getByLabel('超时（分钟）',{exact:true}).fill('8');
 await ordinaryPanel.getByRole('checkbox',{name:'失败后继续',exact:true}).check();
 let ordinarySource=await page.evaluate(()=>window.canvasDraft.definition);
 let ordinaryWorkflow=parseDocument(ordinarySource).toJS();
 assert.equal(ordinaryWorkflow.jobs.build.steps[0].run,'echo from-form');
 assert.equal(ordinaryWorkflow.jobs.build.steps[0]['timeout-minutes'],8);
 assert.equal(ordinaryWorkflow.jobs.build.steps[0].env.MODE,'production');
 assert.equal(ordinaryWorkflow.jobs.build.steps[0]['continue-on-error'],true);
 assert.equal(ordinaryWorkflow.jobs.build.steps[0]['working-directory'],'./app');
 assert.deepEqual(ordinaryWorkflow.jobs.build['x-unknown'],{keep:true});
 assert.deepEqual(ordinaryWorkflow.jobs.build.steps[0]['x-unknown'],{keep:true});
 assert(ordinarySource.includes('# keep form comment'));
 await page.getByRole('tab',{name:'高级 YAML',exact:true}).click();
 await page.locator('.monaco-editor').waitFor();
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 assert.equal(await page.evaluate(()=>window.canvasDraft.definition),ordinarySource,'switching views never rewrites the definition');
 console.log('PASS ordinary step form -> YAML, typed timeout, environment/conditions, unknown fields and comments, seamless view switching');
 await page.evaluate(draft=>window.setCanvasDraft(draft),initialDraft);
 const stepFixtures=[{id:'first',name:'First step',run:'echo first'},{id:'middle',name:'Middle checkout',uses:'actions/checkout@v4'},{id:'last',name:'Last step',run:'echo last'}];
 const deletionFixture={definition:stringify({name:'Deletion context',on:'workflow_dispatch',jobs:{build:{name:'Deletion job','runs-on':'self-hosted',steps:stepFixtures}}}),executor_bindings:[{job_id:'build',target_kind:'host',target_ref:'local',image:'host'}],checkout_bindings:[{job_id:'build',step_id:'middle',connection_id:'42'}]};
 for(const index of [0,1,2]) {
  await page.evaluate(draft=>window.setCanvasDraft(draft),deletionFixture);
  await page.waitForFunction(source=>window.canvasDraft.definition===source&&document.querySelector('.react-flow__node[data-id="build"]')?.textContent.includes('Middle checkout'),deletionFixture.definition);
  await page.waitForTimeout(250); // ReactFlow's measurement/fit debounce must finish before clicking a moving node.
  await page.locator('.react-flow__node[data-id="build"]').getByRole('button',{name:new RegExp(stepFixtures[index].name)}).click();
  const panel=page.getByRole('complementary',{name:'步骤配置',exact:true});
  const remove=panel.getByRole('button',{name:`删除步骤 ${stepFixtures[index].name}`,exact:true});
  assert.equal(await remove.getAttribute('type'),'button','Step delete cannot submit a pipeline form');
  assert.equal(await remove.locator('../..').getByRole('heading',{name:'步骤配置',exact:true}).count(),1,'Step delete belongs to its settings header');
  const beforeSubmit=await page.evaluate(()=>window.canvasSubmits);
  await remove.click();
  const dialog=page.getByRole('dialog');
  await dialog.getByRole('heading',{name:new RegExp(`Deletion job / ${stepFixtures[index].name}`)}).waitFor();
  await dialog.getByRole('button',{name:'取消',exact:true}).click();
  assert.equal(await page.evaluate(()=>window.canvasDraft.definition),deletionFixture.definition,'Cancelling step deletion preserves the draft');
  await remove.click();
  await page.getByRole('button',{name:'确认删除步骤',exact:true}).click();
  const next=await page.evaluate(()=>window.canvasDraft);
  assert.deepEqual(parseDocument(next.definition).toJS().jobs.build.steps,stepFixtures.filter((_,position)=>position!==index),'Delete removes only the selected first/middle/last step');
  assert.equal(next.checkout_bindings.length,index===1?0:1,'Checkout binding removal follows the deleted step identity');
  assert.deepEqual(next.executor_bindings,deletionFixture.executor_bindings);
  assert.equal(await page.evaluate(()=>window.canvasSubmits),beforeSubmit,'Deletion must not submit the surrounding form');
  assert.equal(await page.evaluate(()=>window.canvasInvalid),false,'Remaining valid steps retain a valid draft');
 }
 for(const width of [1280,390,320]) {
  await page.setViewportSize({width,height:900});
  await page.evaluate(draft=>window.setCanvasDraft(draft),deletionFixture);
  await page.waitForFunction(source=>window.canvasDraft.definition===source,deletionFixture.definition);
  await page.waitForTimeout(250);
  await page.locator('.react-flow__node[data-id="build"]').getByRole('button',{name:/First step/}).click();
  const panel=page.getByRole('complementary',{name:'步骤配置',exact:true});
  const heading=panel.getByRole('heading',{name:'步骤配置',exact:true});
  await heading.scrollIntoViewIfNeeded();
  const [headerBox,deleteBox,panelBox]=await Promise.all([heading.boundingBox(),panel.getByRole('button',{name:'删除步骤 First step',exact:true}).boundingBox(),panel.boundingBox()]);
  assert(deleteBox.x>=panelBox.x&&deleteBox.x+deleteBox.width<=panelBox.x+panelBox.width+1,`Delete stays within the ${width}px panel`);
  assert(Math.abs(deleteBox.y+deleteBox.height/2-headerBox.y-headerBox.height/2)<4,`Delete shares the ${width}px title row`);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`No delete header overflow at ${width}px`);
  await page.screenshot({path:`${screenshots}/step-header-${width}.png`});
 }
 await page.setViewportSize({width:1280,height:900});
 await page.evaluate(draft=>window.setCanvasDraft(draft),initialDraft);
 await page.getByRole('button',{name:'任务 1',exact:true}).first().click();
 await page.locator('button[title="在此任务后添加串行任务"]').click();
 assert.match(await page.evaluate(()=>window.canvasDraft.definition),/needs:\n\s+- build/);
 await page.locator('button[title="添加并行任务"]').last().click();
 await page.waitForFunction(()=>document.querySelectorAll('.react-flow__edge').length===2,{},{timeout:5000});
 assert.equal(await page.locator('.react-flow__edge').count(),2);
 await page.getByLabel('流水线画布',{exact:true}).scrollIntoViewIfNeeded();
 await page.waitForTimeout(250);
 const dependencyPath = page.locator('.react-flow__edge-interaction').first();
 await dependencyPath.scrollIntoViewIfNeeded();
 const edgePoint=await dependencyPath.evaluate(path=>{
  // The insertion button intentionally covers the midpoint of short edges.
  for (const fraction of [.1,.9,.2,.8]) {
   const point=path.getPointAtLength(path.getTotalLength()*fraction);
   const screen=new DOMPoint(point.x,point.y).matrixTransform(path.getScreenCTM());
   if (document.elementFromPoint(screen.x,screen.y)?.closest('.react-flow__edge')===path.closest('.react-flow__edge')) return{x:screen.x,y:screen.y};
  }
  return null;
 });
 assert(edgePoint,'Dependency path has a visible click target outside its insertion button');
 await page.mouse.click(edgePoint.x,edgePoint.y);
 const unlink=page.getByRole('button',{name:'删除依赖连接',exact:true});
 await unlink.waitFor();
 assert.equal(await page.getByLabel('流水线画布',{exact:true}).getByRole('button',{name:'删除依赖连接',exact:true}).count(),1,'Dependency delete belongs to the canvas context');
 const [unlinkBox,canvasBox]=await Promise.all([unlink.boundingBox(),page.getByLabel('流水线画布',{exact:true}).boundingBox()]);
 assert(unlinkBox.y>=canvasBox.y&&unlinkBox.y+unlinkBox.height<=canvasBox.y+60,'Dependency action sits in the canvas top-right toolbar');
 const edgeDraft=await page.evaluate(()=>window.canvasDraft.definition);
 const submitsBeforeEdge=await page.evaluate(()=>window.canvasSubmits);
 await unlink.click();
 await page.getByRole('dialog').getByRole('button',{name:'取消',exact:true}).click();
 assert.equal(await page.evaluate(()=>window.canvasDraft.definition),edgeDraft);
 assert.equal(await page.evaluate(()=>window.canvasSubmits),submitsBeforeEdge);
 await page.screenshot({path:`${screenshots}/dependency-action-1280.png`});
 const serialBox=await page.locator('.react-flow__node[data-id="job_1"]').boundingBox(),parallelBox=await page.locator('.react-flow__node[data-id="job_2"]').boundingBox();
 assert(parallelBox.y>=serialBox.y+serialBox.height-1,'parallel task should be placed below its sibling');
 await page.getByRole('button',{name:'添加步骤',exact:true}).last().click();
 await page.getByRole('menuitem',{name:'执行命令',exact:true}).click();
 await page.locator('summary').filter({hasText:'高级 YAML'}).evaluate(el=>{el.parentElement.open=true});
 await page.getByRole('button',{name:'步骤 YAML',exact:true}).click();
 await page.getByRole('dialog').locator('.monaco-editor .view-lines').click();
 await page.keyboard.press('ControlOrMeta+A');
 await page.evaluate(() => navigator.clipboard.writeText('name: Checkout fixture\nuses: actions/checkout@v4\n'));
 await page.keyboard.press('ControlOrMeta+V');
 await page.getByRole('dialog').getByRole('button',{name:'应用到工作流',exact:true}).click();
 await page.getByText('Repository selector').click();
 assert.equal((await page.evaluate(()=>window.canvasDraft.checkout_bindings)).length,1);
 await page.getByRole('button',{name:/^删除步骤/}).click();
 await page.getByRole('button',{name:'确认删除步骤',exact:true}).click();
 assert.equal((await page.evaluate(()=>window.canvasDraft.checkout_bindings)).length,0);
 await page.getByRole('button',{name:'新任务',exact:true}).last().click();
 await page.getByRole('checkbox',{name:'任务 1',exact:true}).uncheck();
 await page.waitForFunction(()=>document.querySelectorAll('.react-flow__edge').length===1,{},{timeout:5000});
 assert.equal(await page.locator('.react-flow__edge').count(),1);
 await page.getByRole('button',{name:'任务 1',exact:true}).first().click();
 const beforeCycle=await page.evaluate(()=>window.canvasDraft.definition);
 await page.getByRole('checkbox',{name:'新任务',exact:true}).first().click();
 await page.getByRole('alert').filter({hasText:'循环'}).waitFor();
 assert.equal(await page.evaluate(()=>window.canvasDraft.definition),beforeCycle);
 await page.reload();
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 await page.getByRole('button',{name:'任务 1',exact:true}).first().click();
 await page.waitForFunction(()=>document.querySelectorAll('.react-flow__edge').length===1,{},{timeout:5000});
 assert.equal(await page.locator('.react-flow__edge').count(),1);
 assert.equal(await page.evaluate(()=>window.canvasDraft.definition),beforeCycle);
 for(const width of [1280,900,390]) {await page.setViewportSize({width,height:900});await page.waitForTimeout(350);assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`overflow at ${width}`);const canvas=await page.getByLabel('流水线画布',{exact:true}).boundingBox();assert(canvas.width>200);for(const node of await page.locator('.react-flow__node').all()){const box=await node.boundingBox();assert(box.x>=canvas.x-1 && box.x+box.width<=canvas.x+canvas.width+1,`node clipped at ${width}: ${JSON.stringify({box,canvas})}`);}}
 await page.getByLabel('任务执行器',{exact:true}).selectOption('local');
 assert.match(await page.getByLabel('任务执行器',{exact:true}).locator('option:checked').textContent(),/Local host/);
 assert.equal((await page.evaluate(()=>window.canvasDraft.executor_bindings))[0].target_ref,'local');
 await page.getByLabel('执行目标类型',{exact:true}).selectOption('host-group');
 assert.equal(await page.getByLabel('任务执行器',{exact:true}).inputValue(),'');
 await page.getByLabel('任务执行器',{exact:true}).selectOption('group');
 assert.deepEqual((await page.evaluate(()=>window.canvasDraft.executor_bindings))[0],{job_id:'build',target_kind:'host-group',target_ref:'group',image:'host'});
 await page.getByLabel('任务类型',{exact:true}).selectOption('approval');
 await page.getByRole('dialog').getByRole('button',{name:/转为人工确认/}).click();
 assert.match(await page.evaluate(()=>window.canvasDraft.definition),/ferry-approval: true/);
 assert.deepEqual(await page.evaluate(()=>window.canvasDraft.executor_bindings),[]);
 await page.getByLabel('任务类型',{exact:true}).selectOption('actions');
 await page.getByRole('button',{name:/步骤1/}).first().click();
 await page.locator('summary').filter({hasText:'高级 YAML'}).evaluate(el=>{el.parentElement.open=true});
 await page.getByRole('button',{name:'步骤 YAML',exact:true}).click();
 await page.getByRole('dialog').locator('.monaco-editor .view-lines').click();
 await page.keyboard.press('ControlOrMeta+A');
 await page.evaluate(() => navigator.clipboard.writeText('name: Setup Node\nuses: actions/setup-node@v4\nwith:\n  node-version: "20"\n'));
 await page.keyboard.press('ControlOrMeta+V');
 await page.getByRole('dialog').getByRole('button',{name:'应用到工作流',exact:true}).click();
 assert.match(await page.evaluate(()=>window.canvasDraft.definition),/node-version: ['"]?20/);
 await page.evaluate(()=>window.setCanvasDraft({...window.canvasDraft,definition:'jobs:\n  first:\n    runs-on: self-hosted\n    steps:\n      - id: source\n        uses: actions/checkout@v4\n  second:\n    runs-on: self-hosted\n    steps:\n      - run: echo kept\n',checkout_bindings:[{job_id:'first',step_id:'source',connection_id:'42'}],executor_bindings:[{job_id:'first',target_ref:'local'}]}));
 await page.getByRole('tab',{name:'高级 YAML',exact:true}).click();
 await page.locator('.monaco-editor').waitFor();
 const setYaml=async value=>{await page.locator('.monaco-editor .view-lines').click();await page.keyboard.press('ControlOrMeta+A');await page.evaluate(text=>navigator.clipboard.writeText(text),value);await page.keyboard.press('ControlOrMeta+V');};
 await setYaml('jobs: [');
 await page.waitForFunction(()=>window.canvasDraft.definition==='jobs: [');
 await page.getByRole('alert').first().waitFor();
 await page.waitForFunction(()=>!!document.querySelector('[class*="squiggly"]'),{},{timeout:10000});
 assert(workers.some(url=>url.includes('yaml.worker')),'YAML language worker did not start');
 assert.equal((await page.evaluate(()=>window.canvasDraft.checkout_bindings)).length,1);
 await setYaml('jobs:\n  second:\n    runs-on: self-hosted\n    steps:\n      - run: echo kept\n');
 await page.waitForFunction(()=>window.canvasDraft.definition==='jobs:\n  second:\n    runs-on: self-hosted\n    steps:\n      - run: echo kept\n');
 assert.deepEqual(await page.evaluate(()=>window.canvasDraft.checkout_bindings),[]);
 assert.deepEqual(await page.evaluate(()=>window.canvasDraft.executor_bindings),[]);
 const imported='name: Imported\non: workflow_dispatch\njobs:\n  imported:\n    runs-on: self-hosted\n    steps:\n      - run: echo imported\n';
 await page.getByLabel('选择 Actions YAML 文件').setInputFiles({name:'workflow.yml',mimeType:'text/yaml',buffer:Buffer.from(imported)});
 await page.waitForFunction(source=>window.canvasDraft.definition===source,imported);
 await page.setViewportSize({width:1280,height:900});
 await page.getByRole('tab',{name:'可视化',exact:true}).click();
 await page.evaluate(()=>window.setCanvasDraft({definition:'jobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo build\n  occupied:\n    needs: build\n    runs-on: self-hosted\n    steps:\n      - run: echo occupied\n',graph_layout:{build:{x:24,y:32},occupied:{x:384,y:32}}}));
 await page.locator('.react-flow__node[data-id="occupied"]').waitFor();
 assert.deepEqual(await page.locator('.react-flow__node[data-id="build"] .border-t button').allTextContents(),['并行','串行']);
 await page.locator('.react-flow__node[data-id="build"] button[title="在此任务后添加串行任务"]').click();
 const added=page.locator('.react-flow__node[data-id="job_1"]');await added.waitFor();
 await page.waitForTimeout(400);
 const buildBox=await page.locator('.react-flow__node[data-id="build"]').boundingBox(),occupiedBox=await page.locator('.react-flow__node[data-id="occupied"]').boundingBox(),addedBox=await added.boundingBox();
 assert(addedBox.x>=buildBox.x+buildBox.width-1,'serial task should be placed to the right of its parent');
 assert(addedBox.y>=occupiedBox.y+occupiedBox.height-1 || addedBox.x>=occupiedBox.x+occupiedBox.width-1,'new task overlaps an existing task');
 await added.locator('button[title="删除任务"]').click();
 await page.getByRole('button',{name:'确认删除任务',exact:true}).click();
 await added.waitFor({state:'detached'});
 assert(!String(await page.evaluate(()=>window.canvasDraft.definition)).includes('job_1'),'card delete did not remove the task');
 // User-facing selectors and references edit the same YAML document.
 const customRunner='runs-on=${{ github.run_id }}/family=c8a+m8a/cpu=4/ram=8+16/image=ubuntu24-full-x64/extras=s3-cache/volume=100gb:gp3';
 await page.evaluate(definition=>window.setCanvasDraft({definition,executor_bindings:[],checkout_bindings:[]}),stringify({jobs:{build:{'runs-on':customRunner,outputs:{version:'${{ steps.compile.outputs.version }}'},steps:[{id:'bundle',uses:'actions/upload-artifact@v4',with:{name:'build-output',path:'dist'}}]},consume:{needs:'build','runs-on':'self-hosted',steps:[{id:'fetch',name:'Download bundle',uses:'actions/download-artifact@v4'},{name:'Inspect result',run:'echo DONE'}]}}}));
 await page.getByRole('button',{name:/Download bundle/}).click();
 const artifactName=page.getByRole('combobox',{name:'with.name',exact:true});
 await artifactName.focus();await page.getByRole('option',{name:'build-output',exact:true}).click();
 assert.equal(parseDocument(await page.evaluate(()=>window.canvasDraft.definition)).toJS().jobs.consume.steps[0].with.name,'build-output');
 await page.getByRole('button',{name:/Inspect result/}).click();
 const shell=page.getByRole('combobox',{name:'命令解释器（Shell）',exact:true});
 await shell.selectOption('powershell');
 assert.equal(await shell.inputValue(),'powershell');
 await shell.selectOption('custom');
 await page.getByRole('textbox',{name:'自定义解释器',exact:true}).fill('bash -e {0}');
 const condition=page.getByRole('combobox',{name:'运行条件（if）',exact:true});
 assert.equal(await condition.inputValue(),'');
 assert.equal(await page.getByRole('textbox',{name:'自定义表达式（高级）',exact:true}).count(),0);
 for (const value of ['failure()','always()','!cancelled()']) {
   await condition.selectOption(value);
   assert.equal(parseDocument(await page.evaluate(()=>window.canvasDraft.definition)).toJS().jobs.consume.steps[1].if,value);
 }
 await condition.selectOption('custom');
 const expression=page.getByRole('textbox',{name:'自定义表达式（高级）',exact:true});
 await expression.fill("github.ref == 'refs/heads/main'");
 assert.equal(parseDocument(await page.evaluate(()=>window.canvasDraft.definition)).toJS().jobs.consume.steps[1].if,"github.ref == 'refs/heads/main'");
 await condition.selectOption('');
 assert(!Object.hasOwn(parseDocument(await page.evaluate(()=>window.canvasDraft.definition)).toJS().jobs.consume.steps[1],'if'));
 const command=page.getByLabel('执行命令',{exact:true});await command.focus();await command.press('End');
 await page.getByRole('button',{name:'变量助手',exact:true}).click();
 const helper=page.getByRole('region',{name:'变量助手',exact:true});
 await helper.getByRole('button').filter({hasText:'steps.fetch.download-path'}).click();
 let rendered=parseDocument(await page.evaluate(()=>window.canvasDraft.definition)).toJS();
 assert.equal(rendered.jobs.consume.steps[1].shell,'bash -e {0}');
 assert(rendered.jobs.consume.steps[1].run.includes("${{ steps.fetch.outputs['download-path'] }}"));
 assert.equal(rendered.jobs.build['runs-on'],customRunner);
 await helper.getByRole('button',{name:'关闭',exact:true}).click();
 await page.getByRole('button',{name:'工作流设置',exact:true}).click();
 const defaultShell=page.getByRole('combobox',{name:'命令解释器（Shell）',exact:true});
 await defaultShell.selectOption('bash');
 rendered=parseDocument(await page.evaluate(()=>window.canvasDraft.definition)).toJS();
 assert.equal(rendered.defaults.run.shell,'bash');
 assert.equal(rendered.jobs.build['runs-on'],customRunner);
 await page.screenshot({path:screenshots+'/outputs-and-shell.png'});
 assert.deepEqual(errors,[]);
 assert.deepEqual(failed,[]);
 console.log('PASS: step delete in card header at1280/390/320, first/middle/last deletion and cancel, no form submission, checkout cleanup, serial/parallel edges, dependency removal, cycle rejection, save/reload and 1280/900/390 layout');
} finally {await browser?.close();await new Promise((resolve,reject)=>server.close(error=>error?reject(error):resolve()));}
