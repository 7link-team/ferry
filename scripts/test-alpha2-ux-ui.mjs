/** Real server, SMTP and optional isolated Redis acceptance for route/refresh/template UX. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,open,readFile,writeFile,access} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createServer,createConnection} from 'node:net';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'alpha2-ux-'));
const freePort=async()=>{const s=createServer();await new Promise(r=>s.listen(0,'127.0.0.1',r));const p=s.address().port;await new Promise(r=>s.close(r));return p;};
const port=await freePort(),origin=`http://127.0.0.1:${port}`;
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE','FERRY_REDIS_URL','FERRY_REDIS_HOST'])delete env[key];
const report={work,checks:[]};let server,browser,page,redis;
const messages=[],sockets=new Set();
const smtp=createServer(socket=>{
  sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.setEncoding('utf8');socket.write('220 localhost ESMTP\r\n');
  let buffer='',data=false,lines=[];
  socket.on('data',chunk=>{buffer+=chunk;let end;while((end=buffer.indexOf('\r\n'))>=0){const line=buffer.slice(0,end);buffer=buffer.slice(end+2);
    if(data){if(line==='.') {messages.push(lines.join('\r\n'));lines=[];data=false;socket.write('250 queued\r\n');}else lines.push(line);}
    else if(/^EHLO|^HELO/i.test(line))socket.write('250-localhost\r\n250 8BITMIME\r\n');
    else if(/^DATA/i.test(line)){data=true;socket.write('354 continue\r\n');}
    else if(/^QUIT/i.test(line))socket.end('221 bye\r\n');else socket.write('250 OK\r\n');
  }});
});
const log=await open(join(work,'server.log'),'w');
try {
  let redisBinary=process.env.FERRY_TEST_REDIS_BINARY || (process.platform==='win32'?resolve('data/maintenance/redis-local/bin/Redis-8.10.3-Windows-x64-msys2/redis-server.exe'):'redis-server');
  let hasRedis=process.platform!=='win32';if(!hasRedis)try{await access(redisBinary);hasRedis=true;}catch{}
  if(hasRedis){
    const redisPort=await freePort();env.FERRY_REDIS_URL=`redis://127.0.0.1:${redisPort}/0`;
    redis=spawn(redisBinary,['--bind','127.0.0.1','--port',String(redisPort),'--save','','--appendonly','no','--dir',work],{windowsHide:true,stdio:['ignore',log.fd,log.fd]});
    for(let i=0;;i++){try{await new Promise((r,j)=>{const s=createConnection(redisPort,'127.0.0.1');s.on('error',j);s.on('connect',()=>{s.end();r();});});break;}catch{assert(i<100);await new Promise(r=>setTimeout(r,100));}}
    await new Promise((r,j)=>{const s=createConnection(redisPort,'127.0.0.1');let response='';s.on('error',j);s.on('data',chunk=>{response+=chunk;if((response.match(/\+OK\r\n/g)||[]).length===300){s.end();r();}});s.on('connect',()=>{s.write(Array.from({length:300},(_,i)=>`SET ux:${i} value\r\n`).join(''));});});
  }
  server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
  for(let i=0;;i++){assert.equal(server.exitCode,null);try{if((await fetch(origin+'/api/status')).ok)break;}catch{}assert(i<200);await new Promise(r=>setTimeout(r,100));}
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const admin=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:900}}),regular=await browser.newContext({locale:'zh-CN'});
  const api=async(context,path,data,method=data?'POST':'GET')=>{const r=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});assert.equal(r.status(),200,await r.text());return r.json();};
  await api(admin,'/setup',{email:'admin@example.invalid',name:'UX administrator',password:'temporary-ux-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  const workspace=await api(admin,'/workspaces',{name:'UX acceptance',slug:'ux-acceptance'});
  await api(regular,'/users',{email:'member@example.invalid',name:'UX member',password:'temporary-ux-password'});
  const memberWorkspace=await api(regular,'/workspaces',{name:'Member workspace',slug:'member-ux'});
  page=await admin.newPage();const errors=[];page.on('pageerror',e=>errors.push(String(e)));
  await page.route('**/api/platform/monitor/redis?*',async route=>{await new Promise(r=>setTimeout(r,500));await route.continue();});
  await page.goto(origin+'/zh-CN/platform/monitor/redis?tab=keys');
  await page.locator('[data-slot="loading-skeleton"]').first().waitFor();
  if(hasRedis){
    const input=page.getByLabel('键匹配规则',{exact:true});await input.waitFor();
    let scans=0;page.on('request',request=>{if(request.url().includes('/api/platform/monitor/redis?'))scans++;});
    await input.fill('ux:*');await input.press('End');await input.press('ArrowLeft');await new Promise(r=>setTimeout(r,600));
    assert.equal(scans,0,'typing a pattern must not scan');assert(await input.evaluate(el=>el===document.activeElement));
    const response=page.waitForResponse(r=>r.url().includes('/api/platform/monitor/redis?'));
    await page.getByRole('button',{name:'搜索',exact:true}).click();
    assert.equal(await input.count(),1);assert.equal(await page.locator('[data-slot="loading-skeleton"]').count(),0,'refresh keeps existing page');
    const first=await(await response).json();assert.notEqual(String(first.cursor),'0','isolated Redis requires multiple SCAN batches');
    const next=page.getByRole('button',{name:'继续扫描',exact:true});await next.waitFor();
    const seen=new Set(first.keys.map(row=>row.key));let cursor=String(first.cursor);
    for(let i=0;cursor!=='0';i++){
      assert(i<30);await next.waitFor({state:'visible'});await page.waitForFunction(()=>[...document.querySelectorAll('button')].some(b=>b.textContent==='继续扫描'&&!b.disabled));
      const pending=page.waitForResponse(r=>r.url().includes('/api/platform/monitor/redis?'));
      await next.click();const data=await(await pending).json();data.keys.forEach(row=>seen.add(row.key));cursor=String(data.cursor);
    }
    assert.equal(seen.size,300);await page.getByText('扫描完成 · 没有下一批',{exact:true}).waitFor();assert(await next.isDisabled());
    await page.reload();await input.waitFor();assert.equal(await input.inputValue(),'ux:*');assert(new URL(page.url()).searchParams.get('tab')==='keys');
    report.checks.push('Real isolated Redis: typing retains focus and causes zero scans; search retains data; Next SCAN traverses all 300 keys; cursor zero explicitly shows completion; URL restores filter/tab');
  } else {await page.getByRole('tab',{name:'键浏览器',exact:true}).waitFor();report.checks.push('Redis not installed: only initial skeleton covered');}
  // Both roles use the same independent create/join URL, including direct refresh and back.
  for(const [context,id] of [[admin,workspace.id],[regular,memberWorkspace.id]]){
    const p=await context.newPage();await p.goto(`${origin}/zh-CN/workspaces/${id}/repository-events`);
    await p.getByRole('button',{name:/workspace|acceptance/i}).last().click();
    await p.getByRole('menuitem',{name:'创建/加入工作空间',exact:true}).click();
    await p.getByText('创建工作空间',{exact:true}).waitFor();assert.equal(new URL(p.url()).pathname,'/zh-CN/workspaces');
    await p.reload();await p.getByText('创建工作空间',{exact:true}).waitFor();
    await p.goBack();await p.getByRole('heading',{name:'仓库事件',exact:true}).waitFor();assert(new URL(p.url()).pathname.endsWith('/repository-events'));
    await p.close();
  }
  report.checks.push('Superadmin and regular user create/join URL, direct refresh and browser back restore correct repository events page');
  // Exercise history state on a long real table layout, with bounded read-only fixture rows.
  const fixtureUser=(await api(admin,'/platform/users')).items[0];assert(fixtureUser);
  await page.route('**/api/platform/users?*',async route=>{
    const result=await route.fetch(),data=await result.json();
    await route.fulfill({json:{...data,items:Array.from({length:70},(_,i)=>({...fixtureUser,id:String(9000+i),name:`Scroll user ${i}`,email:`scroll${i}@example.invalid`})),total:70}});
  });
  await page.goto(origin+'/zh-CN/platform/users?page_size=100&q=Scroll');
  await page.getByText('Scroll user 69',{exact:true}).waitFor();
  const scrollElement=await page.getByText('Scroll user 69',{exact:true}).evaluateHandle(el=>{let node=el;while(node&&!/(auto|scroll)/.test(getComputedStyle(node).overflowY))node=node.parentElement;return node;});
  await scrollElement.evaluate(el=>{el.scrollTop=500;});
  await page.waitForFunction(()=>Number(new URLSearchParams(location.search).get('at'))>=400);
  await page.reload();await page.getByText('Scroll user 69',{exact:true}).waitFor();
  const restored=await page.getByText('Scroll user 69',{exact:true}).evaluate(el=>{let node=el;while(node&&!/(auto|scroll)/.test(getComputedStyle(node).overflowY))node=node.parentElement;return node.scrollTop;});
  assert(restored>=400);assert.equal(await page.getByLabel('姓名或邮箱',{exact:true}).inputValue(),'Scroll');
  await page.getByRole('link',{name:'工作空间',exact:true}).first().click();await page.goBack();
  await page.getByText('Scroll user 69',{exact:true}).waitFor();assert(new URL(page.url()).searchParams.get('q')==='Scroll');
  report.checks.push('Long table: filter/page size/scroll persisted in URL, reload restores offset, navigation and browser back preserve state');
  const empty=await admin.request.post(`${origin}/api/workspaces/${workspace.id}/workflows`,{headers:{'X-Ferry-Request':'1'},data:{name:'Empty trigger acceptance',definition:JSON.stringify({engine:'ferry-graph',schema:1,nodes:[],connections:[]})}});
  assert.equal(empty.status(),201,await empty.text());const workflow=await empty.json();
  await page.goto(`${origin}/zh-CN/workspaces/${workspace.id}/workflows/${workflow.id}/edit`);
  await page.getByRole('button',{name:'测试运行',exact:true}).click();
  const dialog=page.getByRole('dialog');await dialog.getByRole('button',{name:'添加手动触发节点',exact:true}).waitFor();
  assert(await dialog.getByRole('button',{name:'确认',exact:true}).isDisabled());assert.equal(await dialog.getByRole('combobox',{name:'入口节点',exact:true}).count(),0);
  await dialog.getByRole('button',{name:'添加手动触发节点',exact:true}).click();
  await page.locator('.react-flow__node').waitFor();await page.getByRole('button',{name:'保存',exact:true}).click();
  await page.waitForResponse(r=>r.url().endsWith(`/workflows/${workflow.id}`)&&r.request().method()==='GET');
  await page.keyboard.press('Escape');
  await page.getByRole('button',{name:'测试运行',exact:true}).click();
  const entry=dialog.getByRole('combobox',{name:'入口节点',exact:true});await entry.waitFor();assert(await entry.inputValue());
  assert(await dialog.getByRole('button',{name:'确认',exact:true}).isEnabled());await dialog.getByRole('button',{name:'取消',exact:true}).click();
  report.checks.push('Missing manual entry shows explanation and Add action instead of an empty selector; saving the added trigger selects a valid default and enables run');
  await new Promise(r=>smtp.listen(0,'127.0.0.1',r));
  const patch=async values=>{const settings=await api(admin,'/platform/settings/1');return api(admin,'/platform/settings/1',{...settings,...values},'PATCH');};
  await patch({mail_host:'127.0.0.1',mail_port:smtp.address().port,mail_security:'none',mail_from:'noreply@example.invalid'});
  await api(admin,'/platform/mail-test',{email:'recipient@example.invalid'});
  assert.match(messages.at(-1),/multipart\/alternative/);assert.match(messages.at(-1),/text\/plain/);assert.match(messages.at(-1),/text\/html/);
  assert.match(messages.at(-1).replace(/=\r\n/g,''),/max-width:600px/);
  await page.goto(origin+'/zh-CN/platform/settings/1/edit');await page.getByRole('button',{name:'邮箱配置',exact:true}).click();
  await page.getByRole('tab',{name:'邮件模板',exact:true}).click();
  await page.getByRole('textbox',{name:'富文本值',exact:true}).waitFor();
  const preview=page.frameLocator('iframe[title="预览（示例数据）"]');await preview.getByText('123456',{exact:false}).waitFor();
  assert(new URL(page.url()).searchParams.has('form_tab'));await page.reload();await page.getByRole('tab',{name:'邮件模板',exact:true}).waitFor();
  await page.getByRole('textbox',{name:'富文本值',exact:true}).waitFor();
  await page.locator('iframe').scrollIntoViewIfNeeded();
  await page.screenshot({path:join(work,'mail-template-desktop.png')});
  await page.setViewportSize({width:390,height:844});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.screenshot({path:join(work,'mail-template-mobile.png')});
  await patch({mail_test_html:'<h2>Custom delivery</h2><p>Hello {{email}}</p>'});await api(admin,'/platform/mail-test',{email:'recipient@example.invalid'});
  assert.match(messages.at(-1).replace(/=\r\n/g,''),/Custom delivery/);
  await page.getByLabel('模板类型',{exact:true}).selectOption('test');
  await page.getByRole('tab',{name:'HTML',exact:true}).click();
  assert.equal(new URL(page.url()).searchParams.get('mail_template'),'test');
  assert.equal(new URL(page.url()).searchParams.get('mail_editor'),'html');
  await page.reload();await page.getByLabel('模板类型',{exact:true}).waitFor();
  assert.equal(await page.getByLabel('模板类型',{exact:true}).inputValue(),'test');
  assert.equal(await page.getByRole('tab',{name:'HTML',exact:true}).getAttribute('aria-selected'),'true');
  assert.equal(errors.length,0,errors.join('\n'));
  report.checks.push('SMTP multipart HTML + plain fallback; styled default, rich editor and rendered preview; template tab survives reload; custom HTML delivered; 390px no overflow');
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(work,'failure.png')});await writeFile(join(work,'failure.html'),await page.content());}throw error;}
finally{await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));await browser?.close();server?.kill();redis?.kill();for(const s of sockets)s.destroy();if(smtp.listening)await new Promise(r=>smtp.close(r));await log.close();}
