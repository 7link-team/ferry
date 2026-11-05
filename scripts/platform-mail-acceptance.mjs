import {createServer} from 'node:net';
import {execFileSync} from 'node:child_process';
import {join} from 'node:path';
import assert from 'node:assert/strict';

export async function verifyMail({browser,admin,regular,page,origin,api,patchSettings,work,report}) {
 const messages=[];const sockets=new Set();let reject=false;
 const smtp=createServer(socket=>{
  sockets.add(socket);socket.on('close',()=>sockets.delete(socket));socket.setEncoding('utf8');socket.write('220 localhost ESMTP test\r\n');
  let pending='',data=false,lines=[];
  socket.on('data',chunk=>{pending+=chunk;let end;while((end=pending.indexOf('\r\n'))>=0){const line=pending.slice(0,end);pending=pending.slice(end+2);
   if(data){if(line==='.') {data=false;messages.push(lines.join('\r\n'));lines=[];socket.write('250 queued\r\n');}else lines.push(line);continue;}
   if(/^EHLO|^HELO/i.test(line))socket.write('250-localhost\r\n250 8BITMIME\r\n');
   else if(/^MAIL|^RCPT/i.test(line))socket.write(reject?'550 rejected\r\n':'250 OK\r\n');
   else if(/^DATA/i.test(line)){data=true;socket.write('354 send data\r\n');}
   else if(/^QUIT/i.test(line)){socket.end('221 bye\r\n');}
   else socket.write('250 OK\r\n');
  }});
 });
 await new Promise(resolve=>smtp.listen(0,'127.0.0.1',resolve));const port=smtp.address().port;
 const sql=(query,args=[])=>execFileSync('python',['-c','import sqlite3,sys,json; c=sqlite3.connect(sys.argv[1]); rows=c.execute(sys.argv[2],json.loads(sys.argv[3])).fetchall();c.commit();print(json.dumps(rows))',join(work,'ferry.sqlite'),query,JSON.stringify(args)],{encoding:'utf8',windowsHide:true}).trim();
 const body=message=>{let text=message.split('\r\n\r\n').slice(1).join('\r\n\r\n');if(/Content-Transfer-Encoding: base64/i.test(message))text=Buffer.from(text.replace(/\s/g,''),'base64').toString();else text=text.replace(/=\r\n/g,'').replace(/=([A-Fa-f0-9]{2})/g,(_,n)=>String.fromCharCode(parseInt(n,16)));return text;};
 const code=()=>body(messages.at(-1)).match(/\b\d{6}\b/)?.[0];
 const register=(email,verification_code,expected=400)=>api(regular,'/users',{email,password:'temporary-registration-password',name:'Verified member',verification_code},'POST',expected);
 try {
  await api(regular,'/platform/mail-test',{email:'test@example.invalid'},'POST',403);
  await patchSettings({mail_port:0},400);await patchSettings({mail_host:'bad\r\nhost'},400);await patchSettings({mail_security:'invalid'},400);
  await page.goto(origin+'/zh-CN/platform/settings/1/edit');await page.getByRole('button',{name:'邮箱配置',exact:true}).click();
  await page.getByRole('checkbox',{name:'启用邮箱',exact:true}).check();
  await page.getByLabel('SMTP 服务器',{exact:true}).fill('127.0.0.1');await page.getByLabel('SMTP 端口',{exact:true}).fill(String(port));
  await page.getByRole('combobox',{name:'传输加密',exact:true}).click();await page.getByRole('option',{name:'不加密（仅可信网络）',exact:true}).click();
  await page.getByLabel('发件人邮箱',{exact:true}).fill('noreply@example.invalid');
  const saved=page.waitForResponse(r=>r.url()===origin+'/api/platform/settings/1'&&r.request().method()==='PATCH');
  await page.getByRole('button',{name:'保存',exact:true}).click();
  assert.equal((await saved).status(),200);
  await page.getByRole('button',{name:'邮箱配置',exact:true}).click();
  const configured=await api(admin,'/platform/settings/1');assert.equal(configured.mail_enabled,true);assert.equal(configured.mail_verified_at,null);
  assert.equal((await api(regular,'/setup')).email_verification_required,false,'untested config must not gate registration');
  reject=true;await api(admin,'/platform/mail-test',{email:'test@example.invalid'},'POST',400);reject=false;
  assert.equal((await api(admin,'/platform/settings/1')).mail_verified_at,null);
  await page.getByLabel('测试收件邮箱',{exact:true}).fill('operator@example.invalid');
  const testResponse=page.waitForResponse(r=>r.url()===origin+'/api/platform/mail-test'&&r.request().method()==='POST');
  await page.getByRole('button',{name:'发送验证',exact:true}).click();
  assert.equal((await testResponse).status(),200);
  await page.waitForFunction(()=>!document.body.innerText.includes('尚未验证'));
  assert(messages.length>=1);assert.equal((await api(regular,'/setup')).email_verification_required,true);
  assert((await api(admin,'/platform/settings/1')).mail_verified_at>0);
  await page.screenshot({path:join(work,'mail-settings.png')});
  const email='verified@example.invalid';await register(email,'');
  await api(regular,'/registration-code',{email});const firstCode=code();assert.match(firstCode,/^\d{6}$/);
  assert(JSON.parse(sql('select code_hash from email_codes where id=?',['code:'+email]))[0][0].startsWith('$argon2id$'),'code must be stored as an Argon2 hash');
  await api(regular,'/registration-code',{email},'POST',429);
  const wrong=firstCode==='000000'?'111111':'000000';
  for(let i=0;i<5;i++)await register(email,wrong);
  await register(email,firstCode);assert.equal(JSON.parse(sql('select count from email_codes where id=?',['code:'+email]))[0][0],5);
  sql('update email_codes set updated_at=0 where id=?',['code:'+email]);
  await api(regular,'/registration-code',{email});const freshCode=code();
  sql('update email_codes set expires_at=0 where id=?',['code:'+email]);await register(email,freshCode);
  await api(regular,'/registration-code',{email});const validCode=code();await register('wrong-recipient@example.invalid',validCode);await register(email,validCode,200);
  assert.equal(JSON.parse(sql('select count(*) from email_codes where id=?',['code:'+email]))[0][0],0);
  await register(email,validCode);
  // The registration form obtains the current server policy and completes a real SMTP challenge.
  const context=await browser.newContext({locale:'zh-CN'});const registration=await context.newPage();
  await registration.goto(origin+'/zh-CN/login');await registration.getByRole('button',{name:'还没有账号？注册',exact:true}).click();
  await registration.getByLabel('姓名',{exact:true}).fill('UI member');await registration.getByLabel('邮箱',{exact:true}).fill('ui-verified@example.invalid');await registration.getByLabel('密码',{exact:true}).fill('temporary-registration-password');
  await registration.getByRole('button',{name:'发送验证码',exact:true}).click();await registration.getByRole('status').getByText('验证码已发送，请查收邮箱。',{exact:true}).waitFor();
  await registration.getByLabel('邮箱验证码',{exact:true}).fill(code());await registration.screenshot({path:join(work,'mail-registration.png')});
  await registration.getByRole('button',{name:'注册',exact:true}).click();await registration.getByText('创建工作空间',{exact:true}).first().waitFor();await context.close();
  await api(regular,'/registration-code',{email:'pending@example.invalid'});const pendingCode=code();
  const bucket=Math.floor(Date.now()/3600000);sql('update email_codes set count=20 where id=?',['rate:ip:127.0.0.1:'+bucket]);
  await api(regular,'/registration-code',{email:'limited@example.invalid'},'POST',429);sql('update email_codes set count=0 where id=?',['rate:ip:127.0.0.1:'+bucket]);
  await patchSettings({mail_password:'smtp-test-secret'});const secretSettings=await api(admin,'/platform/settings/1');
  assert.equal(secretSettings.mail_has_password,true);assert(!('mail_password' in secretSettings));assert(!sql('select mail_config from platform_state').includes('smtp-test-secret'));
  assert.equal(secretSettings.mail_verified_at,null);assert.equal((await api(regular,'/setup')).email_verification_required,false,'changed credentials invalidate verification');
  await api(admin,'/platform/mail-test',{email:'operator@example.invalid'});await register('pending@example.invalid',pendingCode);
  await patchSettings({mail_enabled:false});
  assert.equal((await api(regular,'/setup')).email_verification_required,false);
  await register('ordinary@example.invalid','',200);await api(regular,'/registration-code',{email:'ordinary@example.invalid'},'POST',403);
  report.checks.push('SMTP real protocol send/failure and UI test; enabled+verified policy; encrypted secret; server-required OTP; send cooldown/5 attempts/expiry/single use; real registration form; changed credentials invalidate verification; disabled mail restores registration');
 }finally {for(const socket of sockets)socket.destroy();await new Promise(resolve=>smtp.close(resolve));}
}
