/** Isolated real server + SMTP acceptance for the mail template workspace. */
import assert from 'node:assert/strict';
import {spawn,execFileSync} from 'node:child_process';
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
  const api=async(path,data,method=data?'POST':'GET',expected=200,locale='zh-CN')=>{const response=await admin.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':locale},...(data?{data}:{})});assert.equal(response.status(),expected,await response.text());return response.json();};
  await api('/setup',{email:'admin@example.invalid',name:'Template acceptance',password:'temporary-template-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  await api('/workspaces',{name:'Template acceptance',slug:'template-acceptance'});
  await new Promise(r=>smtp.listen(0,'127.0.0.1',r));
  const patch=async values=>api('/platform/settings/1',{...await api('/platform/settings/1'),...values},'PATCH');
  await patch({mail_host:'127.0.0.1',mail_port:smtp.address().port,mail_security:'none',mail_from:'noreply@example.invalid'});
  page=await admin.newPage();const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.goto(origin+'/zh-CN/platform/settings/1/edit');
  const expectedGroups=['账号注册','登录防护','监控采集','主机代理','邮箱配置','文件存储','仓库关联','页脚设置','页面脚本'];
  const groups=page.locator('nav').filter({has:page.getByRole('button',{name:'账号注册',exact:true})});
  await page.getByRole('button',{name:'账号注册',exact:true}).waitFor();
  const groupNames=await groups.getByRole('button').allTextContents();
  assert.equal(groupNames.length,expectedGroups.length);
  for(let index=0;index<expectedGroups.length;index++)assert.equal(groupNames[index],expectedGroups[index]);
  await page.getByRole('tab',{name:'邮件模板',exact:true}).click();
  assert.equal(await page.getByLabel('模板类型',{exact:true}).count(),0);
  assert.equal(await page.getByLabel('模板语言',{exact:true}).locator('option').count(),6);
  const insert=async key=>{await page.getByRole('button',{name:'插入变量',exact:true}).click();await page.getByRole('menuitem').filter({hasText:`{{${key}}}`}).click();};
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
  await insert('email');
  assert.equal(await subject.inputValue(),'Before {{email}}After');
  await frame.getByRole('heading',{name:'Before user@example.comAfter',exact:true}).waitFor();
  await rich.focus();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.insertText('Before After');
  await rich.press('ControlOrMeta+Home',{delay:100});for(let i=0;i<7;i++)await rich.press('ArrowRight',{delay:100});
  report.richSelection=await rich.evaluate(element=>{const s=window.getSelection();return {anchor:s?.anchorOffset,focus:s?.focusOffset,text:s?.anchorNode?.textContent,node:s?.anchorNode?.nodeName,html:element.innerHTML,active:element===document.activeElement};});
  assert.equal(report.richSelection.anchor,7,'Place caret between Before and After before inserting');
  await insert('email');
  assert.equal(await rich.innerText(),'Before {{email}}After');
  await frame.getByText('Before user@example.comAfter',{exact:true}).last().waitFor();
  await rich.focus();await page.keyboard.press('ControlOrMeta+A');
  await page.locator('button[data-menu-key="fontSize"]').click();
  await page.locator('li[data-value="24px"]').click();
  assert((await rich.innerHTML()).includes('font-size: 24px'),'Selected text retains the chosen font size');
  await frame.locator('[style*="font-size: 24px"]').waitFor();
  report.checks.push('Native rich-text font-size picker changes selected text and preserves formatting in the HTML email preview');
  report.checks.push('Subject and rich-text variables inserted at caret; preview updates with example data');
  await page.getByRole('tab',{name:'HTML',exact:true}).click();
  const code=page.locator('.monaco-editor textarea').first();await code.waitFor();
  await code.focus();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.insertText('<p>Before After</p>');
  await page.keyboard.press('ControlOrMeta+Home');for(let i=0;i<10;i++)await page.keyboard.press('ArrowRight');
  await insert('email');
  await frame.getByText('Before user@example.comAfter',{exact:true}).last().waitFor();
  await page.getByRole('tab',{name:'富文本',exact:true}).click();
  assert.equal(await rich.innerText(),'Before {{email}}After');
  report.checks.push('Monaco variable insertion preserves HTML and survives rich-text switch');
  await page.getByRole('button',{name:'使用默认模板',exact:true}).click();
  await frame.getByText('123456',{exact:false}).waitFor();
  await subject.fill('CN {{email}}');
  await page.getByLabel('模板语言',{exact:true}).selectOption('en-US');await rich.waitFor();
  assert.equal(await subject.inputValue(),'Registration verification code');
  await subject.fill('EN {{email}}');await rich.focus();await page.keyboard.press('ControlOrMeta+A');await page.keyboard.insertText('Hello {{email}} code {{code}} expires {{expires_minutes}}');
  await page.getByLabel('模板语言',{exact:true}).selectOption('zh-CN');assert.equal(await subject.inputValue(),'CN {{email}}');
  await page.getByLabel('模板语言',{exact:true}).selectOption('ms-MY');await rich.waitFor();assert.equal(await subject.inputValue(),'Kod pengesahan pendaftaran');
  await subject.fill('MS {{email}}');
  await page.getByLabel('模板语言',{exact:true}).selectOption('en-US');await rich.waitFor();assert.equal(await subject.inputValue(),'EN {{email}}');
  assert.equal(await rich.innerText(),'Hello {{email}} code {{code}} expires {{expires_minutes}}');
  const sql=query=>JSON.parse(execFileSync('python',['-c','import sqlite3,sys,json; print(json.dumps(sqlite3.connect(sys.argv[1]).execute(sys.argv[2]).fetchall()))',join(work,'ferry.sqlite'),query],{encoding:'utf8',windowsHide:true}));
  const decode=message=>JSON.parse(execFileSync('python',['-c','import email,email.policy,json,sys; m=email.message_from_string(sys.stdin.read(),policy=email.policy.default); print(json.dumps({"subject":str(m["Subject"]),"parts":[p.get_content() for p in m.walk() if p.get_content_type() in ("text/plain","text/html") and not p.is_multipart()]}))'],{input:message,encoding:'utf8',windowsHide:true}));
  await page.getByLabel('测试收件邮箱',{exact:true}).fill('recipient@example.invalid');
  const testResponse=page.waitForResponse(response=>response.url().endsWith('/api/platform/mail-test')&&response.request().method()==='POST');
  await page.getByRole('button',{name:'发送验证',exact:true}).click();assert.equal((await testResponse).status(),200);
  await page.getByRole('status').filter({hasText:'SMTP已接受测试邮件'}).waitFor();
  let delivered=decode(messages.at(-1));assert.equal(delivered.subject,'EN recipient@example.invalid');assert(delivered.parts.every(part=>part.includes('Hello recipient@example.invalid code 123456 expires 10')));
  assert.deepEqual(sql("SELECT count(*) FROM email_codes WHERE id LIKE 'code:%'"),[[0]]);
  assert.deepEqual((await api('/platform/settings/1')).mail_code_templates,{},'Testing draft does not save it');
  const save=page.getByRole('button',{name:'保存',exact:true});
  const savedResponse=page.waitForResponse(response=>response.url().endsWith('/api/platform/settings/1')&&response.request().method()==='PATCH');
  await save.click();
  assert.equal((await savedResponse).status(),200);
  const saved=await api('/platform/settings/1');
  assert.equal(saved.mail_code_templates['en-US'].subject,'EN {{email}}');assert.equal(saved.mail_code_templates['zh-CN'].subject,'CN {{email}}');assert.equal(saved.mail_code_templates['ms-MY'].subject,'MS {{email}}');
  await api('/platform/mail-test',{email:'saved@example.invalid',locale:'ms-MY'});assert.equal(decode(messages.at(-1)).subject,'MS saved@example.invalid');
  assert(decode(messages.at(-1)).parts[1].includes('max-width:600px'));
  for(const invalid of [{locale:'fr-FR'},{template:{subject:'Invalid',html:'No verification code'}},{template:{subject:'Invalid\r\nBcc: bad',html:'{{code}}'}}])await api('/platform/mail-test',{email:'rejected@example.invalid',...invalid},'POST',400);
  const beforeInvalid=messages.length;await patch({mail_code_templates:{'fr-FR':{subject:'bad',html:'{{code}}'}}}).then(()=>assert.fail('unsupported language accepted'),error=>assert.match(String(error),/400/));assert.equal(messages.length,beforeInvalid);
  await patch({mail_enabled:true,registration_enabled:true});
  await api('/registration-code',{email:'registration@example.invalid'},'POST',200,'en-US');delivered=decode(messages.at(-1));assert.equal(delivered.subject,'EN registration@example.invalid');assert.match(delivered.parts[0],/code \d{6} expires 10/);
  await page.reload();
  await page.getByRole('tab',{name:'邮件模板',exact:true}).click();
  await subject.waitFor();assert.equal(await subject.inputValue(),'EN {{email}}');
  assert.equal(await page.getByLabel('模板语言',{exact:true}).inputValue(),'en-US');
  await rich.waitFor();assert.equal(await rich.innerText(),'Hello {{email}} code {{code}} expires {{expires_minutes}}');
  for(const width of [1440,900,390,320]){await page.setViewportSize({width,height:1000});await page.getByRole('button',{name:'插入变量',exact:true}).click();const menu=page.getByRole('menu');await menu.waitFor();assert.match(await menu.innerText(),/收件人邮箱/);assert.match(await menu.innerText(),/不是发件人/);const bounds=await menu.boundingBox();assert(bounds.x>=0&&bounds.x+bounds.width<=width+1);await page.screenshot({path:join(work,`mail-menu-${width}.png`),fullPage:true});await page.keyboard.press('Escape');await menu.waitFor({state:'hidden'});await page.waitForFunction(()=>document.activeElement?.textContent?.trim()==='插入变量');assert(await page.getByRole('button',{name:'插入变量',exact:true}).evaluate(el=>el===document.activeElement));}
  report.checks.push('Six language drafts stay independent; persistence/reload and actual registration SMTP select locale; current unsaved draft test creates neither a template nor a usable challenge; invalid language/content rejected; compact accessible variable menu includes recipient descriptions at four widths; settings groups follow approved order');
  assert.equal(errors.length,0,errors.join('\n'));
  if(process.env.TEST_CODE_CONFIG){
    for(const invalid of [0,61,1.5]) await api('/platform/settings/1',{...await api('/platform/settings/1'),mail_code_valid_minutes:invalid},'PATCH',400);
    for(const width of [1440,900,390,320]){
      await page.setViewportSize({width,height:1000});await page.goto(origin+'/zh-CN/platform/settings/1/edit?section='+encodeURIComponent('账号注册')+'&form_tab=registration');
      await page.getByRole('checkbox',{name:'启用邮箱验证码',exact:true}).waitFor();
      assert.equal(await page.getByRole('tab',{name:'账号注册',exact:true}).count(),1,'Only one registration tab for both settings sections');
      const minutes=page.getByLabel('验证码有效时间（分钟）',{exact:true});assert.equal(await minutes.getAttribute('min'),'1');assert.equal(await minutes.getAttribute('max'),'60');
      if(width===1440){await minutes.fill('17');const response=page.waitForResponse(r=>r.url().endsWith('/api/platform/settings/1')&&r.request().method()==='PATCH');await page.getByRole('button',{name:'保存',exact:true}).click();assert.equal((await response).status(),200);}
      else assert.equal(await minutes.inputValue(),'17');
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));await page.screenshot({path:join(work,'registration-code-'+width+'.png'),fullPage:true});
    }
    assert.equal((await api('/platform/settings/1')).mail_code_valid_minutes,17);
    await api('/registration-code',{email:'configured@example.invalid'},'POST',200,'en-US');
    const configuredMail=decode(messages.at(-1));assert(configuredMail.parts.every(part=>part.includes('expires 17')));
    const issued=sql("SELECT expires_at,updated_at FROM email_codes WHERE id='code:configured@example.invalid'")[0];assert.equal(issued[0]-issued[1],17*60000);
    const issuedCode=configuredMail.parts[0].match(/code (\d{6})/)[1];
    await patch({mail_code_valid_minutes:3});assert.deepEqual(sql("SELECT expires_at,updated_at FROM email_codes WHERE id='code:configured@example.invalid'")[0],issued);
    const publicContext=await browser.newContext();const registered=await publicContext.request.post(origin+'/api/users',{headers:{'X-Ferry-Request':'1'},data:{name:'Configured code',email:'configured@example.invalid',password:'configured-code-password',verification_code:issuedCode}});assert.equal(registered.status(),200,await registered.text());
    await api('/registration-code',{email:'new-duration@example.invalid'},'POST',200,'en-US');assert(decode(messages.at(-1)).parts.every(part=>part.includes('expires 3')));assert.equal(sql("SELECT expires_at-updated_at FROM email_codes WHERE id='code:new-duration@example.invalid'")[0][0],3*60000);
    await patch({mail_code_enabled:false});assert.equal((await api('/setup')).email_verification_required,false);await api('/registration-code',{email:'disabled@example.invalid'},'POST',403);
    await patch({mail_code_enabled:true});assert.equal((await api('/setup')).email_verification_required,true);
    report.checks.push('Registration code switch and 1–60 minute validation; four-width UI save/reload; real SMTP/plain+HTML 17→3 minutes; database expiry agrees; already-issued challenge remains valid and consumable after TTL change; disable/enable affects public setup and issuance');
    await publicContext.close();
  }
  report.result='PASS';
}catch(error){report.result='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(work,'failure.png'),fullPage:true}).catch(()=>{});await writeFile(join(work,'failure.html'),await page.content()).catch(()=>{});}throw error;}
finally{await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));await browser?.close();server?.kill();for(const socket of sockets)socket.destroy();if(smtp.listening)await new Promise(r=>smtp.close(r));await log.close();}
