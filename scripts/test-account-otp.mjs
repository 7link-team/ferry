/** Isolated acceptance. Never enrolls an existing deployment's user. */
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { createHmac } from 'node:crypto';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'account-otp-'));
const evidence = resolve('data/maintenance/account-otp'); await mkdir(evidence,{recursive:true});
const listener = createServer(); await new Promise(done=>listener.listen(0,'127.0.0.1',done));
const bind = `127.0.0.1:${listener.address().port}`, origin = `http://${bind}`;
await new Promise(done=>listener.close(done));
const env = {...process.env,FERRY_DATA_DIR:work,FERRY_BIND:bind};
for (const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE']) delete env[key];
const log = await open(join(evidence,'acceptance-server.log'),'a');
const binary = resolve(process.env.FERRY_BINARY || 'target/debug/ferry-server.exe');
let server, browser;
const report={work,checks:[],result:'RUNNING'};
const pass=name=>{report.checks.push(name);console.log('PASS',name);};
const wait=async fn=>{for(let i=0;i<200;i++){if(await fn())return;await new Promise(done=>setTimeout(done,150));}throw Error('Acceptance timed out');};
async function start(){server=spawn(binary,[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});await wait(async()=>{assert.equal(server.exitCode,null);try{return(await fetch(origin+'/api/status')).ok;}catch{return false;}});}
async function stop(){if(server?.exitCode===null){const ended=once(server,'exit');server.kill();await ended;}}
function otp(secret){
  let bits=''; for(const char of secret){const value='ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'.indexOf(char);assert(value>=0);bits+=value.toString(2).padStart(5,'0');}
  const key=Buffer.from(bits.match(/.{8}/g).map(value=>parseInt(value,2)));
  const counter=Buffer.alloc(8);counter.writeBigUInt64BE(BigInt(Math.floor(Date.now()/30000)));
  const hash=createHmac('sha1',key).update(counter).digest(),offset=hash[hash.length-1]&15;
  return ((hash.readUInt32BE(offset)&0x7fffffff)%1000000).toString().padStart(6,'0');
}
const request=context=>async(path,data,method=data?'POST':'GET',status=200)=>{
  const response=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'en-US'},...(data?{data}:{})});
  assert.equal(response.status(),status,`${method} ${path}: ${await response.text()}`);return response;
};
try {
  await start();browser=await chromium.launch({channel:'chrome',headless:true});
  const admin=await browser.newContext(),user=await browser.newContext(),other=await browser.newContext(),fresh=await browser.newContext();
  const a=request(admin),u=request(user),o=request(other),f=request(fresh);
  await a('/setup',{email:'owner@example.invalid',name:'Owner',password:'otp-test-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  let config=await(await a('/platform/settings/1')).json();
  assert.equal(config.otp_enabled,false);assert.equal(config.site_title_prefix_enabled,false);
  await a('/platform/settings/1',{...config,otp_enabled:true},'PATCH');
  await u('/users',{email:'otp@example.invalid',name:'OTP User',password:'otp-test-password'});
  await o('/users',{email:'other@example.invalid',name:'Other User',password:'otp-test-password'});
  await f('/profile/otp',undefined,'GET',401);
  await o('/platform/settings/1',undefined,'GET',403);
  await u('/profile/otp/enrollment',{current_password:'otp-test-password',user_id:'2'},'POST',422);
  pass('Defaults disabled; unauthenticated and non-admin access rejected; injected account IDs rejected');
  const page=await user.newPage(),errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(origin+'/zh-CN/account');await page.getByRole('button',{name:'绑定验证器',exact:true}).click();
  await page.getByLabel('当前密码',{exact:true}).last().fill('otp-test-password');await page.getByRole('dialog').getByRole('button',{name:'确认',exact:true}).click();
  const manual=page.getByLabel('手工绑定密钥',{exact:true});await manual.waitFor();const secret=await manual.inputValue();assert.equal(secret.length,32);
  const status=await(await u('/profile/otp')).json();assert.equal(status.enabled,false);
  assert.equal((await(await o('/profile/otp')).json()).enabled,false);
  for(const width of [1440,900,390,320]) {
    await page.setViewportSize({width,height:900});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    const footer=page.getByRole('dialog').locator('.dialog-footer');await footer.scrollIntoViewIfNeeded();
    const box=await footer.boundingBox();assert(box && box.x>=0 && box.x+box.width<=width && box.y+box.height<=900);
    const buttons=await footer.locator('button').evaluateAll(items=>items.map(item=>({width:item.getBoundingClientRect().width,height:item.getBoundingClientRect().height})));
    await page.screenshot({path:join(evidence,`enrollment-${width}.png`)});
    // Outline and filled theme buttons may differ by their two border pixels.
    assert.equal(buttons.length,2);assert(Math.abs(buttons[0].width-buttons[1].width)<=2.1,JSON.stringify({width,buttons,box,footer:await footer.getAttribute('class')}));assert(buttons.every(button=>button.height>=44));
    assert(buttons.reduce((sum,button)=>sum+button.width,0)>box.width*0.75);
    await page.screenshot({path:join(evidence,`enrollment-${width}.png`)});
  }
  await page.getByRole('dialog').locator('img').screenshot({path:join(evidence,'qr-test.png')});
  const decoded=spawnSync('python',['-c',"import sys,pathlib; sys.path.insert(0,str(pathlib.Path('data/maintenance/account-otp/python-libs').resolve())); import zxingcpp; from PIL import Image; from urllib.parse import urlparse,parse_qs; text=zxingcpp.read_barcode(Image.open('data/maintenance/account-otp/qr-test.png')).text; uri=urlparse(text); assert uri.scheme=='otpauth' and uri.netloc=='totp'; query=parse_qs(uri.query); assert query['secret'][0]==sys.stdin.read().strip(); assert query.get('digits',['6'])==['6'] and query.get('period',['30'])==['30']; print('PASS independent QR decode')"],{input:secret,encoding:'utf8',windowsHide:true});
  assert.equal(decoded.status,0,decoded.stderr);pass('Independent QR decoder confirms otpauth TOTP secret, digits and period');
  await page.getByLabel('六位动态验证码',{exact:true}).fill(otp(secret));
  await page.getByRole('dialog').getByRole('button',{name:'确认',exact:true}).click();
  await page.getByText('保存你的恢复码',{exact:true}).waitFor();
  const codes=await page.getByRole('dialog').locator('code').allTextContents();assert.equal(codes.length,10);
  assert(await page.getByRole('button',{name:'完成',exact:true}).isDisabled());
  const downloadPromise=page.waitForEvent('download');await page.getByRole('button',{name:'下载恢复码',exact:true}).click();
  const download=await downloadPromise;const downloaded=await readFile(await download.path(),'utf8');assert.equal(downloaded,codes.join('\n')+'\n');
  await page.getByLabel('我已安全保存恢复码',{exact:true}).check();await page.getByRole('button',{name:'完成',exact:true}).click();
  assert.equal((await(await u('/profile/otp')).json()).enabled,true);
  const now=await(await a('/platform/settings/1')).json();await a('/platform/settings/1',{...now,otp_enabled:false},'PATCH');
  const passwordOnly=await f('/sessions',{email:'otp@example.invalid',password:'otp-test-password'});
  assert.equal(passwordOnly.headers()['cache-control'],'no-store');assert(!passwordOnly.headers()['set-cookie']);
  let challenge=await passwordOnly.json();assert.equal(challenge.otp_required,true);
  await f('/me',undefined,'GET',401);
  await f('/sessions/otp',{token:challenge.otp_token,code:'wrong'},'POST',400);
  pass('Browser enrollment, four widths, local QR, one-time recovery display/download; password alone cannot sign in even with enrollment disabled');
  const login=await fresh.newPage();login.on('pageerror',error=>errors.push(error.message));
  await login.goto(origin+'/zh-CN/login');await login.locator('input[name="email"]').fill('otp@example.invalid');await login.locator('input[name="password"]').fill('otp-test-password');
  await login.getByRole('button',{name:'登录',exact:true}).click();await login.getByRole('dialog').getByText('验证登录身份',{exact:true}).waitFor();
  for(const width of [1440,900,390,320]) {
    await login.setViewportSize({width,height:900});assert(await login.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await login.screenshot({path:join(evidence,`login-${width}.png`)});
  }
  await login.emulateMedia({colorScheme:'dark'});
  await login.waitForFunction(()=>document.documentElement.classList.contains('dark'));
  await login.waitForTimeout(250); // Let the shared theme color transitions settle before capturing.
  await login.screenshot({path:join(evidence,'login-dark-320.png')});
  const contrast=await login.getByRole('dialog').evaluate(element=>{
    const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const context=canvas.getContext('2d');
    const luminance=color=>{context.fillStyle=color;context.fillRect(0,0,1,1);const rgb=[...context.getImageData(0,0,1,1).data].slice(0,3).map(value=>{value/=255;return value<=0.04045?value/12.92:((value+0.055)/1.055)**2.4;});return rgb[0]*0.2126+rgb[1]*0.7152+rgb[2]*0.0722;};
    const background=luminance(getComputedStyle(element).backgroundColor);
    return [...element.querySelectorAll('h2,label,button[data-variant="link"]')].map(item=>{const foreground=luminance(getComputedStyle(item).color);return (Math.max(background,foreground)+0.05)/(Math.min(background,foreground)+0.05);});
  });
  assert(contrast.length>=2 && contrast.every(value=>value>=4.5),JSON.stringify({contrast}));
  await login.emulateMedia({colorScheme:'light'});
  await login.getByRole('button',{name:'使用恢复码',exact:true}).click();await login.getByLabel('恢复码',{exact:true}).fill(codes[0]);
  await login.getByRole('dialog').getByRole('button',{name:'确认',exact:true}).click();await login.getByRole('dialog').waitFor({state:'hidden'});
  assert((await(await f('/me')).json()).user.email==='otp@example.invalid');
  assert.equal((await(await f('/profile/otp')).json()).recovery_remaining,9);
  const token=(await(await o('/sessions',{email:'otp@example.invalid',password:'otp-test-password'})).json()).otp_token;
  await o('/sessions/otp',{token,code:codes[0],use_recovery:true},'POST',400);
  pass('Real browser password → standalone OTP → recovery sign-in; recovery reuse rejected');
  // Restart confirms encrypted state survives process replacement.
  await stop();await start();
  challenge=await(await a('/sessions',{email:'otp@example.invalid',password:'otp-test-password'})).json();
  await a('/sessions/otp',{token:challenge.otp_token,code:codes[1],use_recovery:true});
  assert.equal((await(await a('/profile/otp')).json()).recovery_remaining,8);
  await a('/profile/otp',{current_password:'wrong-password',code:codes[2],use_recovery:true},'DELETE',400);
  const reset=await(await a('/profile/otp/recovery-codes',{current_password:'otp-test-password',code:codes[2],use_recovery:true})).json();
  await f('/me',undefined,'GET',401);
  challenge=await(await f('/sessions',{email:'otp@example.invalid',password:'otp-test-password'})).json();
  await f('/sessions/otp',{token:challenge.otp_token,code:codes[3],use_recovery:true},'POST',400);
  await a('/profile/otp',{current_password:'otp-test-password',code:reset.recovery_codes[0],use_recovery:true},'DELETE');
  const plain=await f('/sessions',{email:'otp@example.invalid',password:'otp-test-password'});assert(plain.headers()['set-cookie']);assert.equal((await plain.json()).otp_required,undefined);
  pass('Restart persistence, reset invalidates old recovery codes and other sessions, password-protected removal restores normal login');
  // Capture every title transition while the public configuration is delayed.
  const titlePage=await other.newPage();await titlePage.addInitScript(()=>{
    window.__titles=[];new MutationObserver(()=>window.__titles.push(document.title)).observe(document.documentElement,{subtree:true,childList:true,characterData:true});
  });
  await titlePage.route('**/api/platform/footer',async route=>{await new Promise(done=>setTimeout(done,1200));await route.continue();});
  for(let i=0;i<2;i++){
    await titlePage.goto(origin+'/zh-CN/login');await titlePage.waitForTimeout(1600);
    const titles=await titlePage.evaluate(()=>window.__titles);assert(!titles.some(value=>/☀️|🌙/.test(value)));
  }
  pass('Disabled title prefixes never flash across initial HTML, delayed settings and refresh');
  assert.deepEqual(errors,[]);report.result='PASS';
} catch(error) {report.error=error.stack;report.result='FAIL';throw error;}
finally {await writeFile(join(evidence,'acceptance.json'),JSON.stringify(report,null,2));await browser?.close();await stop();await log.close();}
