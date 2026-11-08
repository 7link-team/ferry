import { chromium } from 'playwright';
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import assert from 'node:assert/strict';
const directory = await mkdtemp(join(tmpdir(), 'platform-file-catalog-'));
const png=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jB6kAAAAASUVORK5CYII=','base64');
const free=createServer();await new Promise(r=>free.listen(0,'127.0.0.1',r));const port=free.address().port;await new Promise(r=>free.close(r));
const origin=process.env.FERRY_TEST_ORIGIN||`http://127.0.0.1:${port}`;
const log=await open(join(directory,'server.log'),'w');
const env={...process.env,FERRY_BIND:`127.0.0.1:${port}`,FERRY_DATA_DIR:directory,FERRY_ARTIFACT_STORE:'local',FERRY_SANDBOX_PATH:resolve(`target/debug/ferry-sandbox${process.platform==='win32'?'.exe':''}`)};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE'])delete env[key];
const server=process.env.FERRY_TEST_ORIGIN?undefined:spawn(resolve(`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,stdio:['ignore',log.fd,log.fd],windowsHide:true});
let browser;const checks=[];let root;
try{
  for(let i=0;i<300;i++){try{if((await fetch(origin+'/api/status')).ok)break;}catch{}assert.equal(server?.exitCode??null,null,'server exit');assert(i<299,'startup timeout');await new Promise(r=>setTimeout(r,100));}
  browser=await chromium.launch({channel:process.env.FERRY_BROWSER_CHANNEL||'chrome',headless:true});
  const context=await browser.newContext({viewport:{width:1440,height:960}});
  const headers={'X-Ferry-Request':'1','Accept-Language':'zh-CN'};
  const api=async(path,data,method=data?'POST':'GET',expected=200)=>{const r=await context.request.fetch(origin+'/api'+path,{method,headers,...(data?{data}:{})});assert.equal(r.status(),expected,path+': '+await r.text());return expected===204?undefined:r.json();};
  const pass=name=>{checks.push(name);console.log('PASS',name);};
  if(server)await api('/setup',{name:'Media acceptance',email:'media@example.invalid',password:'temporary-media-password',token:(await readFile(join(directory,'setup-token'),'utf8')).trim()});
  else{const a=JSON.parse(await readFile(process.env.FERRY_TEST_ACCESS_FILE,'utf8'));await api('/sessions',{email:a.email,password:a.password});}
  const ws=await api('/workspaces',{name:'Media AI acceptance',slug:'media-ai-'+Date.now()});root=`/workspaces/${ws.id}`;
  const node=(id,type,params,i)=>({id,name:id,type,type_version:1,params,position:[i*320,100]});
  const definition=specs=>({engine:'ferry-graph',schema:1,nodes:[node('start','core.trigger.manual',{input_schema:{type:'object',properties:{}}},0),...specs.map((s,i)=>node(s.id||'n'+i,s.type,s.params,i+1))],connections:specs.map((s,i)=>({from:{node:i?specs[i-1].id||'n'+(i-1):'start',port:'main',index:0},to:{node:s.id||'n'+i,port:'main',index:0}}))});
  const create=async specs=>api(root+'/workflows',{name:'Media AI '+Date.now(),definition:JSON.stringify(definition(specs))},'POST',201);
  const run=async(row,expected='succeeded')=>{const r=await api(`${root}/workflows/${row.id}/executions`,{mode:'manual',revision:'draft',version:row.version,entry_node_id:JSON.parse(row.definition).nodes.find(node=>node.type==='core.trigger.manual').id,inputs:{}},'POST',201);let done;for(let i=0;i<150;i++){done=await api(`${root}/workflow-executions/${r.id}`);if(['succeeded','failed','cancelled'].includes(done.state))break;await new Promise(r=>setTimeout(r,100));}assert.equal(done.state,expected,JSON.stringify(done));const nodes=await api(`${root}/workflow-executions/${r.id}/node-runs`);return {id:r.id,nodes:nodes.items,output:async name=>{const n=nodes.items.find(n=>n.node_id===name);return api(`${root}/workflow-executions/${r.id}/node-runs/${n.id}/output`);}};};
  const upload=async(row,name,mime,data)=>{const r=await context.request.post(`${origin}/api${root}/workflows/${row.id}/assets`,{headers,multipart:{file:{name,mimeType:mime,buffer:data}}});assert.equal(r.status(),201,await r.text());return r.json();};
  const save=async(row,graph)=>{await api(`${root}/workflows/${row.id}`,{version:row.version,definition:JSON.stringify(graph)},'PATCH');return api(`${root}/workflows/${row.id}`);};

  const nats=await api('/platform/monitor/nats'); assert.equal(nats.configured,false); assert.equal(nats.connected,false);
  let media=await create([]);
  const samples=[['notes.md','text/markdown',Buffer.from('# Real preview\nFile catalog content <script>window.previewInjected=true</script>')],['large.txt','text/plain',Buffer.alloc(300*1024,65)],['empty.txt','text/plain',Buffer.alloc(0)],['pixel.png','image/png',png],['binary.bin','application/octet-stream',Buffer.from([0,1,2,3])]];
  const uploaded=[];
  for(const [name,mime,bytes] of samples)uploaded.push(await upload(media,name,mime,bytes));
  const lim={max_input_bytes:1048576,max_output_bytes:1048576,max_rows:100,max_columns:50};
  media=await save(media,definition([{id:'write',type:'file.encode',params:{format:'json',output_field:'result',output_name:'result.json',limits:lim}}]));
  const execution=await run(media);
  const catalog=await api('/platform/monitor/files/catalog?page_size=100');
  assert(catalog.items.length>=6);
  const find=asset=>catalog.items.find(item=>item.sha256===asset.asset.slice(7));
  for(let i=0;i<uploaded.length;i++){
    const row=find(uploaded[i]);assert(row);assert.equal(row.workflow_id,media.id);
    const content=await context.request.get(`${origin}/api/platform/monitor/files/catalog/${row.id}/content`);
    assert.equal(content.status(),200);assert.deepEqual(await content.body(),samples[i][2]);
    assert.equal(content.headers()['cache-control'],'private, no-store');
  }
  const generated=catalog.items.find(item=>item.run_id===execution.id);assert(generated);assert.equal(generated.workflow_id,media.id);
  const ranged=await context.request.get(`${origin}/api/platform/monitor/files/catalog/${find(uploaded[1]).id}/content`,{headers:{Range:'bytes=0-2'}});
  assert.equal(ranged.status(),206);assert.equal((await ranged.body()).toString(),'AAA');
  pass('Real encrypted content, empty/binary/image/download bytes, Range and execution origin match');
  const anonymous=await browser.newContext();
  assert.equal((await anonymous.request.get(`${origin}/api/platform/monitor/files/catalog/${generated.id}/content`)).status(),401);
  const regular=await browser.newContext();
  const reg=await regular.request.post(origin+'/api/users',{headers,data:{email:'regular@example.invalid',name:'Regular',password:'temporary-regular-password'}});
  assert.equal(reg.status(),200,await reg.text());
  const login=await regular.request.post(origin+'/api/sessions',{headers,data:{email:'regular@example.invalid',password:'temporary-regular-password'}});assert.equal(login.status(),200,await login.text());
  assert.equal((await regular.request.get(`${origin}/api/platform/monitor/files/catalog/${generated.id}/content`)).status(),403);
  assert.equal((await anonymous.request.get(`${origin}/api/platform/monitor/nats`)).status(),401);
  assert.equal((await regular.request.get(`${origin}/api/platform/monitor/nats`)).status(),403);
  const otherWs=await api('/workspaces',{name:'Other',slug:'other'});
  await api(`/workspaces/${otherWs.id}/workflows/${media.id}/assets/${uploaded[0].asset.slice(7)}`,undefined,'GET',404);
  pass('Anonymous 401, regular account 403; workspace asset endpoint still rejects cross-workspace access');
  const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
  await page.goto(origin+'/zh-CN/platform/monitor/files?tab=catalog');
  const row=hash=>page.getByRole('row').filter({has:page.getByText(hash,{exact:true})});
  const md=row(uploaded[0].asset.slice(7));
  await md.getByRole('button',{name:'预览',exact:true}).click();
  await page.locator('[role="dialog"] pre').filter({hasText:'# Real preview'}).waitFor();
  assert.equal(await page.evaluate(()=>window.previewInjected),undefined);
  for(const width of [1440,900,390,320]){
    await page.setViewportSize({width,height:960});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    const dialog=page.getByRole('dialog'),box=await dialog.boundingBox();assert(box.x>=0&&box.x+box.width<=width+1);
    await page.screenshot({path:join(directory,`file-preview-${width}.png`),fullPage:true});
  }
  await page.keyboard.press('Escape');await page.setViewportSize({width:1440,height:960});
  const before=page.url(),popupPromise=page.waitForEvent('popup');
  await md.getByRole('link',{name:'打开来源',exact:true}).click();
  const popup=await popupPromise;await popup.waitForLoadState();assert(popup.url().includes(`/workflows/${media.id}`));const retained=new URL(page.url()),previous=new URL(before);retained.searchParams.delete("at");previous.searchParams.delete("at");assert.equal(retained.href,previous.href);await popup.close();
  const runSource=row(generated.sha256).getByRole('link',{name:'打开来源',exact:true});
  assert((await runSource.getAttribute('href')).endsWith(`?execution=${execution.id}`));
  const runPopupPromise=page.waitForEvent('popup');await runSource.click();const runPopup=await runPopupPromise;await runPopup.waitForLoadState();assert(runPopup.url().includes(`execution=${execution.id}`));await runPopup.close();
  const download=page.waitForEvent('download');await md.getByRole('link',{name:'下载文件',exact:true}).click();const file=await download;assert.deepEqual(await readFile(await file.path()),samples[0][2]);
  await row(uploaded[1].asset.slice(7)).getByRole('button',{name:'预览',exact:true}).click();
  await page.getByRole('dialog').getByText(/256 KiB/).waitFor();assert.equal((await page.locator('[role="dialog"] pre').textContent()).length,256*1024);await page.keyboard.press('Escape');
  await row(uploaded[2].asset.slice(7)).getByRole('button',{name:'预览',exact:true}).click();await page.locator('[role="dialog"] pre').waitFor();assert((await page.locator('[role="dialog"] pre').textContent()).length>0);await page.keyboard.press('Escape');
  await row(uploaded[3].asset.slice(7)).getByRole('button',{name:'预览',exact:true}).click();await page.waitForFunction(()=>document.querySelector('[role="dialog"] img')?.naturalWidth===1);await page.keyboard.press('Escape');
  assert.equal(await row(uploaded[4].asset.slice(7)).getByRole('button',{name:'预览',exact:true}).count(),0);
  await page.route('**/api/platform/monitor/files/catalog/*/content',route=>route.fulfill({status:503,body:'Unavailable'}));
  await md.getByRole('button',{name:'预览',exact:true}).click();await page.getByRole('dialog').getByRole('alert').waitFor();await page.unroute('**/api/platform/monitor/files/catalog/*/content');await page.getByRole('dialog').getByRole('button',{name:'重试',exact:true}).click();await page.locator('[role="dialog"] pre').filter({hasText:'# Real preview'}).waitFor();
  assert.deepEqual(errors,[]);
  pass('Browser: four widths, actual text/image/empty previews, truncation, no script execution, real download, source/execution new tabs preserve catalog, failed fetch retries');
  await writeFile(join(directory,'acceptance.json'),JSON.stringify({result:'PASS',directory,checks},null,2));
  console.log(JSON.stringify({result:'PASS',directory,checks}));
}finally{await browser?.close();server?.kill();await log.close();}
