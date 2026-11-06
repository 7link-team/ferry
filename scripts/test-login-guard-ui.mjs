/** Real HTTP, shared-database login protection and browser acceptance. Uses an isolated database only. */
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const work=await mkdtemp(join(tmpdir(),'login-guard-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));
const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`;
const env={...process.env,FERRY_DATA_DIR:work,FERRY_BIND:`127.0.0.1:${port}`,FERRY_TRUSTED_PROXIES:'127.0.0.1/32'};
for(const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE','FERRY_ARTIFACT_STORE'])delete env[key];
const log=await open(join(work,'server.log'),'w');
const binary=resolve(process.env.FERRY_BINARY||`target/debug/ferry-server${process.platform==='win32'?'.exe':''}`);
let server,browser,page;
const report={work,checks:[]};
const hash=value=>createHash('sha256').update(value).digest('hex');
const email='member@example.invalid',password='temporary-guard-password',ip='203.0.113.10';
function sql(statement,params=[]) {
  const code='import sqlite3,json,sys\nc=sqlite3.connect(sys.argv[1],timeout=15)\nr=c.execute(sys.argv[2],json.loads(sys.argv[3]))\nprint(json.dumps(r.fetchall() if r.description else []))\nc.commit()';
  const result=spawnSync('python',['-c',code,join(work,'ferry.sqlite'),statement,JSON.stringify(params)],{encoding:'utf8',windowsHide:true});
  assert.equal(result.status,0,result.stderr);return JSON.parse(result.stdout);
}
const reset=()=>sql('DELETE FROM login_guards');
const resetRates=()=>sql("DELETE FROM login_guards WHERE id LIKE 'login-%' OR id LIKE 'verify-%' OR id LIKE 'challenge-ip:%'");
async function start() {
  server=spawn(binary,[],{env,stdio:['ignore',log.fd,log.fd],windowsHide:true});
  for(let i=0;;i++) {
    assert.equal(server.exitCode,null);
    try{if((await fetch(origin+'/api/status')).ok)break;}catch{}
    assert(i<200,'startup timeout');await new Promise(r=>setTimeout(r,100));
  }
}
async function stop(){if(server&&server.exitCode===null){const closed=new Promise(r=>server.once('exit',r));server.kill();await closed;}}
async function api(context,path,body,expected=200,method=body?'POST':'GET') {
  const response=await context.request.fetch(origin+'/api'+path,{method,...(body?{data:body}:{}),headers:{'X-Ferry-Request':'1','Accept-Language':'zh-CN'}});
  const text=await response.text();assert.equal(response.status(),expected,`${method} ${path}: ${text}`);
  return text?JSON.parse(text):undefined;
}
function answer(challenge,address=email,source=ip) {
  const bind=hash(`${Buffer.byteLength(address)}:${address}:${source}`);
  const id='captcha:'+bind+':'+hash(challenge.id);
  const rows=sql('SELECT payload FROM login_guards WHERE id=?',[id]);assert.equal(rows.length,1);
  const stored=JSON.parse(rows[0][0]);assert.equal(stored.token_hash,hash(challenge.id));
  assert(!rows[0][0].includes(challenge.id),'raw challenge secret must not be stored');
  // A fixture represents a human solving the pixels; no production answer endpoint exists.
  stored.answer_hash=hash(challenge.id+':42');stored.kind='math';
  sql('UPDATE login_guards SET payload=? WHERE id=?',[JSON.stringify(stored),id]);
  return {login_challenge_id:challenge.id,login_challenge_answer:'42'};
}
try {
  await start();browser=await chromium.launch({headless:true,...(process.platform==='win32'?{channel:'chrome'}:{})});
  const context=async(source=ip)=>browser.newContext({locale:'zh-CN',extraHTTPHeaders:{'x-forwarded-for':source,'X-Ferry-Request':'1'}});
  const admin=await context('203.0.113.1'),member=await context(),other=await context();
  await api(admin,'/setup',{email:'admin@example.invalid',name:'Guard administrator',password,token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  await api(member,'/users',{email,name:'Guard member',password});
  await api(other,'/users',{email:'other@example.invalid',name:'Other member',password});
  await member.clearCookies();await other.clearCookies();
  let settings=await api(admin,'/platform/settings/1');assert.equal(settings.login_guard_mode,'local');
  const login=(client=member,extra={})=>api(client,'/sessions',{email,password,...extra});
  assert.equal((await api(member,'/sessions/challenge',{email})).required,false);
  for(let i=0;i<5;i++)await api(member,'/sessions',{email,password:'wrong-password'},i===4?428:401);
  await api(member,'/sessions',{email,password},428);
  let challenge=await api(member,'/sessions/challenge',{email});
  assert(challenge.required&&challenge.image.startsWith('data:image/bmp;base64,'));
  assert(!('answer' in challenge));
  let proof=answer(challenge);
  const results=await Promise.all([member.request.post(origin+'/api/sessions',{data:{email,password,...proof}}),other.request.post(origin+'/api/sessions',{data:{email,password,...proof}})]);
  assert.deepEqual(results.map(r=>r.status()).sort(),[200,428],'one-use proof is atomic across concurrent clients');
  resetRates();await api(member,'/sessions',{email,password,...proof},428);
  report.checks.push('5th failed password requires server verification; password alone cannot bypass; concurrent proof replay admits exactly one login; proof secrets hashed at rest');

  reset();for(let i=0;i<5;i++)await api(member,'/sessions',{email,password:'bad'},i===4?428:401);
  challenge=await api(member,'/sessions/challenge',{email});proof=answer(challenge);
  await api(other,'/sessions',{email:'other@example.invalid',password,...proof},428);
  const otherIp=await context('203.0.113.11');await api(otherIp,'/sessions',{email,password,...proof},428);
  await login(member,proof);resetRates();
  challenge=await api(member,'/sessions/challenge',{email});proof=answer(challenge);
  sql("UPDATE login_guards SET expires_at=0 WHERE id LIKE 'captcha:%'");
  await api(member,'/sessions',{email,password,...proof},428);
  challenge=await api(member,'/sessions/challenge',{email});proof=answer(challenge);
  await api(member,'/sessions/challenge',{email});await api(member,'/sessions',{email,password,...proof});
  await api(member,'/sessions',{email,password,...proof},428);
  report.checks.push('Proofs reject cross-account, cross-IP, expired and replayed challenges; issuing another tab challenge preserves the first proof');

  resetRates();
  const verifiedProof=async()=>{const c=await api(member,'/sessions/challenge',{email});return api(member,'/sessions/challenge/verify',{email,...answer(c)});};
  let receipt=await verifiedProof();assert(receipt.login_verification_token);
  const storedReceipt=sql('SELECT id,payload FROM login_guards WHERE id=?',['verified:'+hash(receipt.login_verification_token)]);assert.equal(storedReceipt.length,1);assert(!JSON.stringify(storedReceipt).includes(receipt.login_verification_token));
  await api(member,'/sessions/challenge/verify',{email,login_verification_token:receipt.login_verification_token},428);
  const reuse=await Promise.all([member,other].map(c=>c.request.post(origin+'/api/sessions',{data:{email,password,...receipt}})));
  assert.deepEqual(reuse.map(response=>response.status()).sort(),[200,428]);
  resetRates();
  const tabA=await api(member,'/sessions/challenge',{email});
  const tabB=await api(member,'/sessions/challenge',{email});
  await api(member,'/sessions',{email,password},428);
  const receiptA=await api(member,'/sessions/challenge/verify',{email,...answer(tabA)});
  const receiptB=await api(member,'/sessions/challenge/verify',{email,...answer(tabB)});
  await login(member,receiptA);await login(member,receiptB);
  report.checks.push('Independent challenges survive another tab and login requests without proofs; both tabs can verify their own challenge');
  resetRates();receipt=await verifiedProof();await api(otherIp,'/sessions',{email,password,...receipt},428);
  receipt=await verifiedProof();await api(other,'/sessions',{email:'other@example.invalid',password,...receipt},428);
  receipt=await verifiedProof();sql("UPDATE login_guards SET expires_at=0 WHERE id LIKE 'verified:%'");await api(member,'/sessions',{email,password,...receipt},428);
  resetRates();await api(member,'/sessions/challenge/verify',{email,login_challenge_id:'fabricated',login_challenge_answer:'42'},428);
  report.checks.push('Separate verification receipts are hashed, expire, bind account/IP, cannot mint receipts and permit only one concurrent login; forged proof rejected');

  reset();let responses=await Promise.all(Array.from({length:36},(_,i)=>member.request.post(origin+'/api/sessions',{data:{email:`unknown${i}@example.invalid`,password:'bad'}})));
  assert.equal(responses.filter(r=>r.status()===429).length,6,'30 requests per IP per minute');
  reset();responses=await Promise.all(Array.from({length:15},()=>member.request.post(origin+'/api/sessions',{data:{email:'unknown@example.invalid',password:'bad'}})));
  assert.equal(responses.filter(r=>r.status()===429).length,5,'10 requests per account per minute');
  await stop();await start();await api(member,'/sessions',{email:'unknown@example.invalid',password:'bad'},429);
  reset();assert((await login()).id);
  sql('INSERT INTO login_guards(id,count,payload,expires_at) VALUES(?,?,?,?)',['expired-fixture',0,null,0]);
  await api(member,'/sessions/challenge',{email});assert.equal(sql("SELECT count(*) FROM login_guards WHERE id='expired-fixture'")[0][0],0);
  report.checks.push('Concurrent account/IP limits are exact, persist after restart, and recover after expiry; expired rows cleaned');

  reset();settings=await api(admin,'/platform/settings/1');
  await api(member,'/platform/settings/1',{...settings,login_guard_mode:'turnstile'},403,'PATCH');
  await api(admin,'/platform/settings/1',{...settings,login_guard_mode:'turnstile',login_turnstile_site_key:'fixture-public-site-key',login_turnstile_secret_key:'fixture-private-secret',login_turnstile_hostname:'example.invalid'},200,'PATCH');
  settings=await api(admin,'/platform/settings/1');assert(settings.login_turnstile_has_secret);assert(!JSON.stringify(settings).includes('fixture-private-secret'));
  assert(!sql('SELECT login_security FROM platform_state')[0][0].includes('fixture-private-secret'));
  await api(admin,'/platform/settings/1',{...settings,login_turnstile_secret_key:''},200,'PATCH');
  settings=await api(admin,'/platform/settings/1');assert(settings.login_turnstile_has_secret);
  for(let i=0;i<5;i++)await api(member,'/sessions',{email,password:'bad'},i===4?428:401);
  challenge=await api(member,'/sessions/challenge',{email});assert.equal(challenge.kind,'turnstile');assert.equal(challenge.site_key,'fixture-public-site-key');assert(!JSON.stringify(challenge).includes('fixture-private-secret'));
  await api(member,'/sessions',{email,password},428);
  const html=await member.request.get(origin+'/zh-CN/login');const csp=html.headers()['content-security-policy'];
  assert(csp.includes("script-src 'self' https://challenges.cloudflare.com"));assert(csp.includes("frame-src 'self' https://challenges.cloudflare.com"));
  report.checks.push('Only platform admins configure Turnstile; secret encrypted, redacted and retained on blank edit; server requires proof and CSP permits only the official origin');

  await member.clearCookies();page=await member.newPage();const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  await page.route('https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit',route=>route.fulfill({contentType:'application/javascript',body:'window.turnstile={render:function(el,opts){el.textContent="Turnstile fixture";window.guardComplete=function(){opts.callback("fixture-token")};return "fixture"},remove:function(){}}'}));
  await page.goto(origin+'/zh-CN/login');await page.getByLabel('邮箱',{exact:true}).fill(email);await page.getByLabel('密码',{exact:true}).fill('bad');
  await page.getByRole('button',{name:'登录',exact:true}).click();await page.getByText('Turnstile fixture',{exact:true}).waitFor();
  await page.evaluate(()=>window.guardComplete());assert(await page.getByRole('button',{name:'验证并继续',exact:true}).isEnabled());
  await page.screenshot({path:join(work,'turnstile.png')});
  await page.getByRole('button',{name:'取消',exact:true}).click();assert.equal(await page.getByRole('dialog').count(),0);
  settings=await api(admin,'/platform/settings/1');await api(admin,'/platform/settings/1',{...settings,login_guard_mode:'local'},200,'PATCH');resetRates();
  await page.goto(origin+'/zh-CN/login');await page.getByLabel('邮箱',{exact:true}).fill(email);await page.getByLabel('密码',{exact:true}).fill('bad');await page.getByRole('button',{name:'登录',exact:true}).click();
  await page.getByText('安全验证',{exact:true}).waitFor();
  const image=page.locator('img[src^="data:image/bmp;base64,"]');await image.waitFor();assert(await image.evaluate(el=>el.complete&&el.naturalWidth===280));
  await page.screenshot({path:join(work,'local-challenge.png')});
  await page.setViewportSize({width:390,height:844});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));await page.screenshot({path:join(work,'local-challenge-mobile.png')});
  const seekKind=async(kind)=>{
    for(let attempt=0;attempt<40;attempt++){
      await page.waitForFunction(()=>[...document.querySelectorAll('button')].some(button=>button.textContent==='换一个'&&!button.disabled));
      if(await (kind==='slider'?page.getByRole('slider'):page.getByRole('status',{name:'选择顺序',exact:true})).count())return;
      resetRates();await Promise.all([page.waitForResponse(response=>response.url().endsWith('/api/sessions/challenge')&&response.request().method()==='POST'),page.getByRole('button',{name:'换一个',exact:true}).click()]);
      await page.getByRole('button',{name:'换一个',exact:true}).waitFor();
    }
    throw new Error('Random challenge kind not reached: '+kind);
  };
  await seekKind('slider');
  const slider=page.getByRole('slider');const piece=page.locator('span[aria-hidden][style*="left:"]');
  const currentSliderToken=()=>sql("SELECT payload FROM login_guards WHERE id LIKE ? ORDER BY expires_at DESC LIMIT 1",['captcha:'+hash(`${Buffer.byteLength(email)}:${email}:${ip}`)+':%'])[0][0];
  // Use an impossible answer for this deliberate failure without exposing a production answer endpoint.
  const rejectSlider=()=>{const value=JSON.parse(currentSliderToken());const id='captcha:'+hash(`${Buffer.byteLength(email)}:${email}:${ip}`)+':'+value.token_hash;value.answer_hash=hash('unmatchable-fixture');sql('UPDATE login_guards SET payload=? WHERE id=?',[JSON.stringify(value),id]);};
  rejectSlider();
  const imageBeforeNetworkFailure=await image.getAttribute('src');
  await page.route('**/api/sessions/challenge/verify',route=>route.abort('failed'),{times:1});
  await slider.focus();await slider.press('ArrowRight',{delay:20});
  await page.getByRole('alert').waitFor();
  assert.equal(await image.getAttribute('src'),imageBeforeNetworkFailure,'network failure must preserve the current challenge');
  const retryResponse=page.waitForResponse(response=>response.url().endsWith('/api/sessions/challenge/verify'));
  await page.getByRole('button',{name:'验证并继续',exact:true}).click();assert.equal((await retryResponse).status(),428);
  report.checks.push('Network failure retains the challenge and allows explicit retry');
  report.checks.push('Releasing keyboard slider automatically verifies and displays rejection; failed challenge refreshes');
  await seekKind('slider');
  rejectSlider();const topBefore=await piece.evaluate(el=>parseFloat(el.style.top));
  const sliderBox=await page.getByRole('slider').boundingBox();await page.mouse.move(sliderBox.x+10,sliderBox.y+sliderBox.height/2);await page.mouse.down();await page.mouse.move(sliderBox.x+sliderBox.width/2,sliderBox.y+sliderBox.height/2,{steps:6});
  const topMoved=await piece.evaluate(el=>parseFloat(el.style.top));assert(Math.abs(topMoved-topBefore)>3,'piece must move vertically along its slope');
  await page.screenshot({path:join(work,'slider-dragging-mobile.png')});
  const failureResponse=page.waitForResponse(response=>response.url().endsWith('/api/sessions/challenge/verify'));await page.mouse.up();assert.equal((await failureResponse).status(),428);
  await seekKind('select');
  const order=page.getByRole('status',{name:'选择顺序',exact:true});
  assert.equal(await order.getByText('待选',{exact:true}).count(),3);
  const choices=page.locator('button[aria-pressed]');assert.equal(await choices.count(),8);
  const boxes=await choices.evaluateAll(elements=>elements.map(el=>{const r=el.getBoundingClientRect();return {top:r.top,width:r.width,height:r.height};}));
  assert.equal(new Set(boxes.map(box=>box.top)).size,2);assert(boxes.every(box=>box.width>=44&&box.height>=44));
  const selected=[];for(let i=0;i<3;i++){selected.push(await choices.nth(i).innerText());await choices.nth(i).click();assert.equal(await page.getByRole('button',{name:'验证并继续',exact:true}).isEnabled(),i===2);}
  for(let i=0;i<3;i++)assert((await order.locator(':scope > span').nth(i).innerText()).includes(selected[i]));
  await page.screenshot({path:join(work,'selection-order-mobile.png')});
  await page.getByRole('button',{name:'重新选择',exact:true}).click();assert.equal(await order.getByText('待选',{exact:true}).count(),3);assert(await page.getByRole('button',{name:'验证并继续',exact:true}).isDisabled());
  report.checks.push('Verification modal separates login; character choices use two fixed rows, ordered slots, require three selections and clear proof on reset');
  await page.getByRole('button',{name:'取消',exact:true}).click();
  await page.getByLabel('邮箱',{exact:true}).fill('changed@example.invalid');assert.equal(await page.getByText('安全验证',{exact:true}).count(),0,'changing account clears bound proof');
  await page.getByLabel('邮箱',{exact:true}).fill(email);await page.getByLabel('密码',{exact:true}).fill(password);resetRates();await page.getByRole('button',{name:'登录',exact:true}).click();await seekKind('select');
  const correct=[];for(let i=0;i<3;i++){correct.push(await choices.nth(i).innerText());await choices.nth(i).click();}
  const challengeRow=sql("SELECT id,payload FROM login_guards WHERE id LIKE ? ORDER BY expires_at DESC LIMIT 1",['captcha:'+hash(`${Buffer.byteLength(email)}:${email}:${ip}`)+':%'])[0];const challengeKey=challengeRow[0];const stored=JSON.parse(challengeRow[1]);
  // Represent solving the image in this isolated fixture; the production response never exposes its answer.
  await page.route('**/api/sessions/challenge/verify',async route=>{
    const body=route.request().postDataJSON();stored.answer_hash=hash(body.login_challenge_id+':'+correct.join(''));sql('UPDATE login_guards SET payload=? WHERE id=?',[JSON.stringify(stored),challengeKey]);await route.continue();
  });
  const loginSuccess=page.waitForResponse(response=>response.url().endsWith('/api/sessions')&&response.request().method()==='POST');
  await page.getByRole('button',{name:'验证并继续',exact:true}).click();assert.equal((await loginSuccess).status(),200);
  await page.getByRole('dialog').waitFor({state:'detached'});await page.unroute('**/api/sessions/challenge/verify');
  // Reproduce logout/relogin, including a receipt expiring between verify and login.
  await page.route('**/api/sessions/challenge/verify',async route=>{
    const body=route.request().postDataJSON();
    const id='captcha:'+hash(`${Buffer.byteLength(email)}:${email}:${ip}`)+':'+hash(body.login_challenge_id);
    const row=JSON.parse(sql('SELECT payload FROM login_guards WHERE id=?',[id])[0][0]);
    row.answer_hash=hash(body.login_challenge_id+':'+body.login_challenge_answer.toUpperCase());
    sql('UPDATE login_guards SET payload=? WHERE id=?',[JSON.stringify(row),id]);await route.continue();
  });
  for(let cycle=0;cycle<2;cycle++){
    await api(member,'/sessions',undefined,200,'DELETE');resetRates();
    await page.goto(origin+'/zh-CN/login');await page.getByLabel('邮箱',{exact:true}).fill(email);await page.getByLabel('密码',{exact:true}).fill(password);
    await page.getByRole('button',{name:'登录',exact:true}).click();await seekKind('select');
    for(let i=0;i<3;i++)await page.locator('button[aria-pressed]').nth(i).click();
    if(cycle===0)await page.route('**/api/sessions',async route=>{sql("UPDATE login_guards SET expires_at=0 WHERE id LIKE 'verified:%'");await route.continue();},{times:1});
    let response=page.waitForResponse(r=>r.url().endsWith('/api/sessions')&&r.request().method()==='POST');
    await page.getByRole('button',{name:'验证并继续',exact:true}).click();
    assert.equal((await response).status(),cycle===0?428:200);
    if(cycle===0){
      await page.getByRole('dialog').waitFor();await page.getByRole('alert').waitFor();
      await seekKind('select');for(let i=0;i<3;i++)await page.locator('button[aria-pressed]').nth(i).click();
      response=page.waitForResponse(r=>r.url().endsWith('/api/sessions')&&r.request().method()==='POST');
      await page.getByRole('button',{name:'验证并继续',exact:true}).click();assert.equal((await response).status(),200);
    }
    await page.getByRole('dialog').waitFor({state:'detached'});
  }
  report.checks.push('Logout/relogin succeeds repeatedly; an expired verification receipt keeps/reopens the modal and a fresh challenge successfully completes login');
  report.checks.push('Successful independent verification automatically resumes real password login without a second Sign in click');
  const settingsPage=await admin.newPage();await settingsPage.goto(origin+'/zh-CN/platform/settings/1/edit');await settingsPage.getByRole('button',{name:'登录防护',exact:true}).click();await settingsPage.getByRole('combobox',{name:'验证码方式',exact:true}).waitFor();assert((await settingsPage.getByRole('combobox',{name:'验证码方式',exact:true}).textContent()).includes('本地随机验证码'));await settingsPage.screenshot({path:join(work,'login-settings.png')});
  assert.deepEqual(errors,[]);report.checks.push('Actual local captcha raster, refresh/account reset and Turnstile widget, platform settings, 390px layout, no page errors; external Turnstile provider success not claimed');
  const asset=(await readFile('dist/index.html','utf8')).match(/index-[A-Za-z0-9_-]+\.js/)[0];assert((await (await fetch(origin)).text()).includes(asset));
  report.result='PASS';
} catch(error) {
  report.result='FAIL';report.error=String(error);if(page){await page.screenshot({path:join(work,'failure.png')});await writeFile(join(work,'failure.html'),await page.content());}throw error;
} finally {
  await writeFile(join(work,'acceptance.json'),JSON.stringify(report,null,2));console.log('Login guard acceptance:',work,report.result);
  await browser?.close();await stop();await log.close();
}
