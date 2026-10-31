/** Named connection form against HTTP fixtures, including a real OAuth popup/channel. */
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { chromium } from 'playwright';
import { createServer as createNetServer } from 'node:net';
const listener=createNetServer();await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));
const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));
const server=await createServer({server:{host:"127.0.0.1",port,strictPort:true},plugins:[{name:'connection-browser-test',configureServer(server){server.middlewares.use('/__connection',(_req,res)=>{res.setHeader('Content-Type','text/html');res.end(`<html><head><meta name="viewport" content="width=device-width,initial-scale=1"/><script type="module">import RefreshRuntime from '/@react-refresh';RefreshRuntime.injectIntoGlobalHook(window);window.$RefreshReg$=()=>{};window.$RefreshSig$=()=>type=>type;window.__vite_plugin_react_preamble_installed__=true;</script></head><body><div id="root"></div><script type="module" src="/__connection-entry.tsx"></script></body></html>`);});},resolveId(id){if(id==='/__connection-entry.tsx')return id;},load(id){if(id==='/__connection-entry.tsx')return `import React,{useState} from 'react';import {createRoot} from 'react-dom/client';import {RepositorySource} from '/src/components/RepositorySource.tsx';import '/src/style.css';function App(){const[draft,setDraft]=useState({name:'Team source',description:'Shared account',provider:'github',credential_id:'0'});window.connectionDraft=draft;return <main style={{maxWidth:900,padding:24}}><RepositorySource workspace="109124796398174208" draft={draft} readOnly={false} onChange={patch=>setDraft(d=>({...d,...patch}))} onValidityChange={value=>{window.connectionBlocked=value}}/></main>};createRoot(document.getElementById('root')).render(<App/>);`;}}]});
let browser;
try {
 await server.listen();const origin=server.resolvedUrls.local[0].replace(/\/$/,'');browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHROME_PATH?{executablePath:process.env.PLAYWRIGHT_CHROME_PATH}:{})});const context=await browser.newContext();const page=await context.newPage();const errors=[];page.on('pageerror',error=>errors.push(error.message));
 let refreshed=false,authorizeQuery;
 await context.route('**/api/workspaces/109124796398174208/credentials?*',route=>route.fulfill({json:{items:[{id:'11',name:'Saved PAT',kind:'token'},{id:'12',name:'SSH deploy key',kind:'ssh-key'},...(refreshed?[{id:'13',name:'New PAT',kind:'token'}]:[])]}}));
 await context.route('**/api/workspaces/109124796398174208/repository-providers',route=>route.fulfill({json:{items:[{id:'github',web_available:true}]}}));
 await context.route('**/api/workspaces/109124796398174208/repository-providers/github/authorizations?*',route=>{authorizeQuery=new URL(route.request().url()).searchParams;return route.fulfill({json:{url:origin+'/__oauth-callback?channel='+authorizeQuery.get('channel')}});});
 await context.route('**/__oauth-callback?*',route=>route.fulfill({contentType:'text/html',body:`<html><script>window.opener=null;const channel=new BroadcastChannel('repository-authorized-'+new URLSearchParams(location.search).get('channel'));channel.postMessage({type:'repository-authorized',provider:'github',status:'connected',connection_id:'45'});channel.close();</script></html>`}));
 await page.goto(origin+'/__connection');
 await page.getByLabel('凭据',{exact:true}).selectOption('11');
 assert.equal(await page.evaluate(()=>window.connectionDraft.credential_id),'11');assert.equal(await page.locator('input[type=password]').count(),0);
 assert.equal(await page.getByLabel('选择平台仓库',{exact:true}).count(),0);
 const newCredential=page.getByRole('link',{name:'新建凭据',exact:true});assert.equal(await newCredential.getAttribute('target'),'_blank');assert.equal(await newCredential.getAttribute('href'),'/workspaces/109124796398174208/credentials/new?scope=build');
 refreshed=true;await page.getByRole('button',{name:'刷新凭据',exact:true}).click();await page.getByRole('option',{name:'New PAT',exact:true}).waitFor({state:'attached'});assert.equal(await page.getByLabel('凭据',{exact:true}).inputValue(),'11');
 await page.getByLabel('代码平台',{exact:true}).selectOption('gitea');await page.getByLabel('凭据',{exact:true}).selectOption('11');
 await page.getByLabel('平台网站地址',{exact:true}).fill('https://git.example.invalid');await page.getByLabel('平台 API 地址',{exact:true}).fill('https://git.example.invalid/api/v1');
 assert.equal(await page.evaluate(()=>window.connectionDraft.base_url),'https://git.example.invalid');assert.equal(await page.evaluate(()=>window.connectionBlocked),false);
 await page.getByLabel('代码平台',{exact:true}).selectOption('codeup');await page.getByLabel('Codeup 组织 ID',{exact:true}).fill('org-team');assert.equal(await page.evaluate(()=>window.connectionDraft.organization),'org-team');
 await page.getByLabel('代码平台',{exact:true}).selectOption('git');await page.getByRole('option',{name:'SSH deploy key',exact:true}).waitFor({state:'attached'});assert.equal(await page.evaluate(()=>window.connectionBlocked),false);
 await page.getByLabel('代码平台',{exact:true}).selectOption('github');await page.getByLabel('凭据',{exact:true}).selectOption('11');
 await page.getByLabel('平台授权方式',{exact:true}).selectOption('web');
 const popupPromise=context.waitForEvent('page');await page.getByRole('button',{name:'打开网页授权',exact:true}).click();await popupPromise;
 await page.waitForURL('**/workspaces/109124796398174208/code-connections/45');assert.equal(authorizeQuery.get('name'),'Team source');assert.equal(authorizeQuery.get('description'),'Shared account');assert.equal(await page.evaluate(()=>window.connectionDraft.credential_id),'11');
 assert.deepEqual(errors,[]);
 console.log('PASS: existing credential reuse/refresh, no secret/repository inputs, real popup BroadcastChannel callback navigates to named connection');
} finally {await browser?.close();await server.close();}
