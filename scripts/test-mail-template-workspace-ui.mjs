/** Isolated real server + SMTP acceptance for the mail template workspace. */
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp,open,readFile,writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join,resolve} from 'node:path';
import {createServer} from 'node:net';
import {chromium} from 'playwright';

const work=await mkdtemp(join(tmpdir(),'mail-template-workspace-'));
const freePort=async()=>{const server=createServer();await new Promise(r=>server.listen(0,'127.0.0.1',r));const port=server.address().port;await new Promise(r=>server.close(r));return port;};
const port=await freePort(),origin=`http://127.0.0.1:${port}`;
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE','FERRY_REDIS_URL','FERRY_REDIS_HOST'])delete env[key];
let server,browser,page;
const messages=[],sockets=new Set(),report={work,checks:[]};
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
  server=spawn(resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`),[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
  for(let i=0;;i++){assert.equal(server.exitCode,null);try{if((await fetch(origin+'/api/status')).ok)break;}catch{}assert(i<200);await new Promise(r=>setTimeout(r,100));}
  browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const admin=await browser.newContext({locale:'zh-CN',viewport:{width:1440,height:1000}});
  const api=async(path,data,method=data?'POST':'GET',expected=200)=>{const response=await admin.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'},...(data?{data}:{})});assert.equal(response.status(),expected,await response.text());return response.json();};
  await api('/setup',{email:'admin@example.invalid',name:'Template acceptance',password:'temporary-template-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  await api('/workspaces',{name:'Template acceptance',slug:'template-acceptance'});
  await new Promise(r=>smtp.listen(0,'127.0.0.1',r));
  const patch=async values=>api('/platform/settings/1',{...await api('/platform/settings/1'),...values},'PATCH');
  await patch({mail_host:'127.0.0.1',mail_port:smtp.address().port,mail_security:'none',mail_from:'noreply@example.invalid'});
  page=await admin.newPage();const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.goto(origin+'/zh-CN/platform/settings/1/edit');
  await page.getByRole('button',{name:'邮箱配置',exact:true}).click();
  await page.getByRole('tab',{name:'邮件模板',exact:true}).click();
  const subject=page.getByLabel('邮件主题',{exact:true});
  const rich=page.getByRole('textbox',{name:'富文本值',exact:true});
  await rich.waitFor();
  const frame=page.frameLocator('iframe[title="预览（示例数据）"]');
  await frame.getByText('123456',{exact:false}).waitFor();
  for(const width of [1440,900,390,320]){
    await page.setViewportSize({width,height:1000});
    const columns=page.locator('[data-mail-template-columns]');
    const left=await columns.locator(':scope > section').nth(0).boundingBox();
    const right=await columns.locator(':scope > section').nth(1).boundingBox();
    assert(left&&right);
    if(width===1440)assert(right.x>left.x+left.width-1&&Math.abs(right.y-left.y)<2,'Desktop preview beside editor');
    else assert(right.y>left.y+left.height-1,'Narrow preview below editor');
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'No page horizontal overflow');
    await page.screenshot({path:join(work,`mail-${width}.png`),fullPage:true});
    if(width<1200){
      await page.locator('iframe').scrollIntoViewIfNeeded();
      await page.screenshot({path:join(work,`mail-${width}-preview.png`),fullPage:true});
    }
  }
  report.checks.push('Styled defaults and 1440/900/390/320 responsive split/stack without page overflow');
  await page.setViewportSize({width:1440,height:1000});
  await subject.fill('Before After');await subject.press('Home');for(let i=0;i<7;i++)await subject.press('ArrowRight');
  await page.getByRole('button',{name:'{{email}}',exact:true}).click();
  assert.equal(await subject.inputValue(),'Before {{email}}After');
  await frame.getByRole('heading',{name:'Before user@example.comAfter',exact:true}).waitFor();
  await rich.focus();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.insertText('Before After');
  await rich.press('ControlOrMeta+Home',{delay:100});for(let i=0;i<7;i++)await rich.press('ArrowRight',{delay:100});
  report.richSelection=await rich.evaluate(element=>{const s=window.getSelection();return {anchor:s?.anchorOffset,focus:s?.focusOffset,text:s?.anchorNode?.textContent,node:s?.anchorNode?.nodeName,html:element.innerHTML,active:element===document.activeElement};});
  assert.equal(report.richSelection.anchor,7,'Place caret between Before and After before inserting');
  await page.getByRole('button',{name:'{{email}}',exact:true}).click();
  assert.equal(await rich.innerText(),'Before {{email}}After');
  await frame.getByText('Before user@example.comAfter',{exact:true}).last().waitFor();
  report.checks.push('Subject and rich-text variables inserted at caret; preview updates with example data');
  await page.getByRole('tab',{name:'HTML',exact:true}).click();
  const code=page.locator('.monaco-editor textarea').first();await code.waitFor();
  await code.focus();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.insertText('<p>Before After</p>');
  await page.keyboard.press('ControlOrMeta+Home');for(let i=0;i<10;i++)await page.keyboard.press('ArrowRight');
  await page.getByRole('button',{name:'{{email}}',exact:true}).click();
  await frame.getByText('Before user@example.comAfter',{exact:true}).last().waitFor();
  await page.getByRole('tab',{name:'富文本',exact:true}).click();
  assert.equal(await rich.innerText(),'Before {{email}}After');
  report.checks.push('Monaco variable insertion preserves HTML and survives rich-text switch');
  await page.getByRole('button',{name:'使用默认模板',exact:true}).click();
  await frame.getByText('123456',{exact:false}).waitFor();
  await page.getByLabel('模板类型',{exact:true}).selectOption('test');
  await rich.waitFor();
  assert.equal(await page.getByRole('button',{name:'{{code}}',exact:true}).count(),0);
  assert.equal(await page.getByRole('button',{name:'{{expires_minutes}}',exact:true}).count(),0);
  await subject.fill('Delivery preview');await rich.focus();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.insertText('Hello {{email}}');
  const save=page.getByRole('button',{name:'保存',exact:true});
  const savedResponse=page.waitForResponse(response=>response.url().endsWith('/api/platform/settings/1')&&response.request().method()==='PATCH');
  await save.click();
  assert.equal((await savedResponse).status(),200);
  const saved=await api('/platform/settings/1');
  assert.equal(saved.mail_test_subject,'Delivery preview');assert(saved.mail_test_html.includes('Hello {{email}}'));
  await api('/platform/mail-test',{email:'recipient@example.invalid'});
  const delivered=messages.at(-1).replace(/=\r\n/g,'');
  assert.match(delivered,/multipart\/alternative/);assert.match(delivered,/text\/html/);assert.match(delivered,/Delivery preview/);assert.match(delivered,/Hello recipient@example.invalid/);assert.match(delivered,/max-width:600px/);
  await page.reload();
  await page.getByRole('tab',{name:'邮件模板',exact:true}).click();
  await subject.waitFor();assert.equal(await subject.inputValue(),'Delivery preview');
  assert.equal(await page.getByLabel('模板类型',{exact:true}).inputValue(),'test');
  await rich.waitFor();assert.equal(await rich.innerText(),'Hello {{email}}');
  report.checks.push('Template kinds keep allowed variables separate; reset, save/read-back and reload retain correct kind/content; actual SMTP HTML + plain fallback');
  assert.equal(errors.length,0,errors.join('\n'));
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(work,'failure.png'),fullPage:true}).catch(()=>{});await writeFile(join(work,'failure.html'),await page.content()).catch(()=>{});}throw error;}
finally{await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));await browser?.close();server?.kill();for(const socket of sockets)socket.destroy();if(smtp.listening)await new Promise(r=>smtp.close(r));await log.close();}
