/** Focused real ReactFlow browser interactions, with no Rust/Go build required. */
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { chromium } from 'playwright';
const server = await createServer({ server: { port: 0 }, plugins: [{ name: 'canvas-test', configureServer(server) {
 server.middlewares.use('/__canvas', (_req, res) => { res.setHeader('Content-Type', 'text/html'); res.end(`<html><head><meta name="viewport" content="width=device-width,initial-scale=1"/><script type="module">import RefreshRuntime from '/@react-refresh';RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;</script></head><body><div id="root"></div><script type="module" src="/__canvas-entry.tsx"></script></body></html>`); });
 }, resolveId(id) { if(id==='/__canvas-entry.tsx') return id; }, load(id) { if(id==='/__canvas-entry.tsx') return `import React, {useState} from 'react';import {createRoot} from 'react-dom/client';import {ActionsEditor} from '/src/components/ActionsEditor.tsx';import {DEFAULT_ACTIONS,editActionStep} from '/src/actions-workflow.ts';import '/src/style.css';function App(){const [draft,setDraft]=useState(JSON.parse(localStorage.getItem('canvas-draft')||'null')||{definition:DEFAULT_ACTIONS});window.canvasDraft=draft;window.setCanvasDraft=setDraft;return <main style={{padding:16,maxWidth:'100%'}}><ActionsEditor draft={draft} options={[{value:'host:local',label:'Local host'}]} readOnly={false} onChange={patch=>setDraft(d=>{const next={...d,...patch};localStorage.setItem('canvas-draft',JSON.stringify(next));return next})} onValidityChange={()=>{}} renderCheckout={(job,index)=> <button onClick={()=>setDraft(d=>({...d,definition:editActionStep(d.definition,job,index,{id:'source'}),checkout_bindings:[{job_id:job,step_id:'source',repository_id:'repo'}]}))}>Repository selector</button>}/></main>};createRoot(document.getElementById('root')).render(<App/>);`; } }] });
let browser;
try {
 await server.listen();browser=await chromium.launch({headless:true});
 const page=await browser.newPage({viewport:{width:1280,height:900}});await page.goto(server.resolvedUrls.local[0]+'__canvas');
 await page.getByRole('button',{name:'构建',exact:true}).first().click();
 await page.getByRole('button',{name:'添加串行任务',exact:true}).click();
 assert.match(await page.evaluate(()=>window.canvasDraft.definition),/needs:\n\s+- build/);
 await page.getByRole('button',{name:'添加并行任务',exact:true}).click();
 assert.equal(await page.locator('.react-flow__edge').count(),2);
 await page.getByRole('button',{name:'添加拉取代码步骤',exact:true}).click();
 await page.getByText('Repository selector').click();
 assert.equal((await page.evaluate(()=>window.canvasDraft.checkout_bindings)).length,1);
 await page.getByRole('button',{name:'删除步骤',exact:true}).click();
 assert.equal((await page.evaluate(()=>window.canvasDraft.checkout_bindings)).length,0);
 await page.getByRole('button',{name:'新任务',exact:true}).last().click();
 await page.getByRole('checkbox',{name:'构建',exact:true}).uncheck();
 assert.equal(await page.locator('.react-flow__edge').count(),1);
 await page.getByRole('button',{name:'构建',exact:true}).first().click();
 const beforeCycle=await page.evaluate(()=>window.canvasDraft.definition);
 await page.getByRole('checkbox',{name:'新任务',exact:true}).first().click();
 await page.getByRole('alert').filter({hasText:'循环'}).waitFor();
 assert.equal(await page.evaluate(()=>window.canvasDraft.definition),beforeCycle);
 await page.reload();
 await page.getByRole('button',{name:'构建',exact:true}).first().click();
 assert.equal(await page.locator('.react-flow__edge').count(),1);
 assert.equal(await page.evaluate(()=>window.canvasDraft.definition),beforeCycle);
 for(const width of [1280,900,390]) {await page.setViewportSize({width,height:900});await page.waitForTimeout(350);assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),`overflow at ${width}`);const canvas=await page.getByLabel('流水线画布',{exact:true}).boundingBox();assert(canvas.width>200);for(const node of await page.locator('.react-flow__node').all()){const box=await node.boundingBox();assert(box.x>=canvas.x-1 && box.x+box.width<=canvas.x+canvas.width+1,`node clipped at ${width}: ${JSON.stringify({box,canvas})}`);}}
 await page.evaluate(()=>window.setCanvasDraft({...window.canvasDraft,target_kind:'host',target_ref:'local'}));
 assert.match(await page.getByLabel('任务执行器',{exact:true}).locator('option:checked').textContent(),/Local host/);
 console.log('PASS: serial/parallel edges, dependency removal, cycle rejection, save/reload, checkout cleanup and 1280/900/390 layout');
} finally {await browser?.close();await server.close();}
