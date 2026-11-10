/** Isolated API and real-browser acceptance; never connects to the running deployment. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { copyFile, mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'site-settings-'));
const output = join(work, 'evidence'); await mkdir(output);
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const bind = `127.0.0.1:${listener.address().port}`, origin = `http://${bind}`;
await new Promise(r => listener.close(r));
const env = {...process.env, FERRY_DATA_DIR:work, FERRY_BIND:bind};
for (const key of ['FERRY_DATABASE_URL','FERRY_NATS_URL','FERRY_CLUSTER_NAMESPACE']) delete env[key];
const log = await open(join(work,'server.log'),'a');
const binary = join(work, `site-test-server${process.platform==='win32'?'.exe':''}`);
await copyFile(resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${process.platform==='win32'?'.exe':''}`), binary);
const report = {work, checks:[]};
const pass = name => {report.checks.push(name); console.log('PASS',name);};
let server, browser;
const wait = async fn => {for(let i=0;i<200;i++){if(await fn())return;await new Promise(r=>setTimeout(r,150));}throw Error('Acceptance timed out');};
async function launch(){
  server=spawn(binary,[],{env,windowsHide:true,stdio:['ignore',log.fd,log.fd]});
  await wait(async()=>{assert.equal(server.exitCode,null);try{return (await fetch(origin+'/api/status')).ok;}catch{return false;}});
}
async function stop(){if(server?.exitCode===null){const ended=once(server,'exit');server.kill();await ended;}}
try {
  await launch();
  const expectedAsset=(await readFile(resolve('dist/index.html'),'utf8')).match(/src="(\/assets\/index-[^"]+\.js)"/)?.[1];
  assert(expectedAsset,'Built frontend entry missing');
  assert((await (await fetch(origin+'/en-US/login')).text()).includes(expectedAsset),'Server binary embeds an older frontend; rebuild ferry-server after npm run build');
  browser=await chromium.launch({channel:'chrome',headless:true});
  const admin=await browser.newContext({viewport:{width:1440,height:1000}});
  const anonymous=await browser.newContext();
  const api = context => async(path,data,method=data?'POST':'GET',status=200)=>{
    const response=await context.request.fetch(origin+'/api'+path,{method,headers:{'X-Ferry-Request':'1','Accept-Language':'en-US'},...(data?{data}:{})});
    assert.equal(response.status(),status,`${method} ${path}: ${await response.text()}`);
    return response.json();
  };
  const request=api(admin), publicRequest=api(anonymous);
  await request('/setup',{name:'Owner',email:'owner@example.invalid',password:'temporary-site-password',token:(await readFile(join(work,'setup-token'),'utf8')).trim()});
  let config=await request('/platform/settings/1');
  assert.equal(config.site_name,''); assert.equal(config.site_title_prefix_enabled,false);
  const savedName='Example <&> Console';
  const patch=async data=>{const result=await request('/platform/settings/1',{...config,...data},'PATCH');config=await request('/platform/settings/1');return result;};
  await patch({site_name:savedName,site_title_prefix_enabled:true,site_title_visible_prefix:'🌞',site_title_hidden_prefix:'🌚',footer_html:'<p>Existing footer</p>',analytics_enabled:false,analytics_js:'window.__siteTest = true;'});
  let view=await publicRequest('/platform/footer');
  assert.equal(view.site_name,savedName);assert.equal(view.content,'<p>Existing footer</p>');
  assert.deepEqual(Object.keys(view).sort(),['content','site_name','site_title_hidden_prefix','site_title_prefix_enabled','site_title_visible_prefix']);
  const html=await (await anonymous.request.get(origin+'/zh-CN/login')).text();
  assert(html.includes('<title>Example &lt;&amp;&gt; Console</title>'));
  pass('API persists site identity; public payload excludes scripts/secrets; first HTML escapes custom name');
  for(const data of [{site_name:'a'.repeat(121)},{site_title_hidden_prefix:'🌙'.repeat(33)},{site_name:'line\nbreak'},{site_title_prefix_enabled:'yes'}]) {
    await request('/platform/settings/1',{...config,...data},'PATCH',400);
    assert.equal((await request('/platform/settings/1')).version,config.version);
  }
  await request('/platform/settings/1',{...config,version:config.version-1,site_name:'stale'},'PATCH',409);
  await publicRequest('/platform/settings/1',undefined,'GET',401);
  const member=await browser.newContext(), memberRequest=api(member);
  await memberRequest('/users',{name:'Reader',email:'reader@example.invalid',password:'temporary-reader-password'});
  await memberRequest('/platform/settings/1',undefined,'GET',403);
  await memberRequest('/platform/settings/1',{...config,site_name:'unauthorized'},'PATCH',403);
  pass('Invalid settings roll back; stale version and non-admin writes are blocked');

  const page=await admin.newPage();
  const path='/en-US/platform/settings/1/edit?section=site';
  await page.goto(origin+path);
  await page.getByLabel('Site name',{exact:true}).waitFor();
  assert.equal(await page.getByRole('tab').count(),4);
  await page.getByLabel('Site name',{exact:true}).fill('Updated Console');
  await page.getByRole('button',{name:'Save',exact:true}).click();
  await wait(async()=> (await publicRequest('/platform/footer')).site_name==='Updated Console');
  await wait(async()=> (await page.title()).endsWith('| Updated Console'));
  config=await request('/platform/settings/1');
  assert.equal(config.footer_html,'<p>Existing footer</p>');assert.equal(config.analytics_js,'window.__siteTest = true;');
  await page.reload();await page.getByLabel('Site name',{exact:true}).waitFor();
  assert.equal(await page.getByLabel('Site name',{exact:true}).inputValue(),'Updated Console');
  await page.getByRole('tab',{name:'Browser tabs',exact:true}).click();
  assert.equal(await page.getByLabel('Visible tab prefix',{exact:true}).inputValue(),'🌞');
  await page.getByLabel('Visible tab prefix',{exact:true}).fill('☀️');
  await page.getByRole('button',{name:'Save',exact:true}).click();
  await wait(async()=> (await page.title()).startsWith('☀️ '));
  pass('Four tabs; real form save updates title immediately; reload restores name/prefix and preserves footer/scripts');

  const login=await anonymous.newPage();await login.goto(origin+'/zh-CN/login');
  await login.getByRole('heading',{name:'Updated Console',exact:true}).waitFor();
  assert((await login.title()).includes('登录 | Updated Console'));
  const workspace=await request('/workspaces',{name:'Site settings test',slug:'site-settings-test'});
  await page.goto(origin+`/en-US/workspaces/${workspace.id}/dashboard`);
  await page.getByRole('link',{name:'Updated Console',exact:true}).waitFor();
  await page.goto(origin+'/en-US/platform/settings/1/edit?section='+encodeURIComponent('页面脚本'));
  await page.getByRole('tab',{name:'Page scripts',exact:true,selected:true}).waitFor();
  assert.equal(new URL(page.url()).searchParams.get('section'),'site');
  pass('Public login and workspace header use configured identity; old script section URL opens the matching site tab');

  for(const locale of ['en-US','zh-CN','ms-MY']) {
    for(const width of [1440,900,390,320]) {
      await page.setViewportSize({width,height:1000});
      await page.goto(origin+`/${locale}/platform/settings/1/edit?section=site&form_tab=tabs`);
      await page.locator('[role="tabpanel"][data-state="active"] input').first().waitFor();
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`${locale}/${width} page overflow`);
      const tabs=page.getByRole('tab');assert.equal(await tabs.count(),4);
      for(let i=0;i<4;i++) {
        const box=await tabs.nth(i).boundingBox(); assert(box.height>=44,`${locale}/${width} tab target`);
      }
      const input=page.locator('input[name="site_title_visible_prefix"]');
      await input.scrollIntoViewIfNeeded();
      const rect=await input.boundingBox();
      assert(rect && rect.x>=0 && rect.x+rect.width<=width,`${locale}/${width} prefix input clipped: ${JSON.stringify(rect)}`);
      await page.screenshot({path:join(output,`${locale}-${width}.png`),fullPage:true});
    }
  }
  pass('English/Chinese/Malay at Desktop/Pad/Phone widths: no page overflow and 44px tab targets (12 screenshots)');

  config=await request('/platform/settings/1');await patch({site_name:'X'.repeat(120)});
  await login.setViewportSize({width:320,height:900});await login.reload();
  await login.getByRole('heading',{name:'X'.repeat(120),exact:true}).waitFor();
  assert.equal(await login.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  for(const target of [login.getByRole('heading',{name:'X'.repeat(120),exact:true}),login.getByRole('button',{name:'登录',exact:true})]) {
    const rect=await target.boundingBox();assert(rect.x>=0&&rect.x+rect.width<=320,`Long-name login control clipped: ${JSON.stringify(rect)}`);
  }
  await login.screenshot({path:join(output,'long-name-login-320.png'),fullPage:true});
  pass('Maximum-length unbroken site name wraps on 320px login');

  config=await request('/platform/settings/1');await patch({site_name:'',site_title_prefix_enabled:false});
  await login.reload();await wait(async()=> (await login.title())==='登录 | AI 工作台');
  for(const [locale,name] of [['en-US','AI Workspace'],['ms-MY','Ruang Kerja AI']]) {
    await login.goto(origin+`/${locale}/login`);await wait(async()=> (await login.title()).endsWith('| '+name)&&!(await login.title()).startsWith('☀️'));
  }
  pass('Empty site name restores localized defaults; disabling prefix removes emoji');
  await stop(); await launch();
  view=await publicRequest('/platform/footer');
  assert.equal(view.site_name,'');assert.equal(view.site_title_prefix_enabled,false);assert.equal(view.content,'<p>Existing footer</p>');
  pass('Configuration and existing footer survive server restart and repeated migration startup');
  report.result='PASS';
} catch(error){report.result='FAIL';report.error=String(error.stack||error);throw error;}
finally {
  await writeFile(join(output,'result.json'),JSON.stringify(report,null,2));
  console.log('Evidence:',output);
  await browser?.close();await stop();await log.close();
}
