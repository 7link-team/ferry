/** Production UI acceptance with delayed, stateful API fixtures; no real Agent side effects. */
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { preview } from 'vite';
import { chromium } from 'playwright';

const evidence = await mkdtemp(join(tmpdir(), 'conversation-pending-'));
const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.FERRY_UI_ORIGIN || `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
try {
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  for (const width of [1440, 900, 390, 320]) {
    const context = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: width < 1200, isMobile: width < 768 });
    const page = await context.newPage(), errors = [], writes = [];
    page.on('pageerror', error => errors.push(error.message));
    const decision = () => ({ tool: 'ferry_request', input: { method: 'POST', path: '/workflows', body: { name: 'Weekly contribution report' } }, decision: null, dispatching: false, sent: false });
    const makeTurn = (id, state, keys) => ({ id, owner_id: '1', state, prompt: 'Prepare report', attachments: [], error: '', usage: null, decisions: Object.fromEntries(keys.map(key => [key, decision()])) });
    const row = { id: '2', title: 'Approval acceptance', kind: 'private', state: 'active', viewer_id: '1', owner_id: '1', can_manage: true,
      members: [{ id: '1', name: 'Owner' }], agents: [], messages: [], version: 1, revision_id: '1', members_version: '1',
      config: { name: 'Report assistant', runtime: 'codex', model: '', cwd: '/tmp' }, binding: { execution_user: 'user', canonical_directory: '/tmp' },
      turns: [makeTurn('10', 'completed', ['a', 'b', 'c', 'd']), makeTurn('20', 'running', ['e', 'f'])] };
    const count = () => row.turns.flatMap(turn => Object.values(turn.decisions)).filter(item => item.decision === null && !item.resolved).length;
    let failDismiss = true, probes = 0;
    let releaseList;
    const initialList = new Promise(resolve => releaseList = resolve);
    let detailGate = Promise.resolve();
    let suspendNextDetail = false, detailSuspended = false, releaseSuspended;
    await page.route('**/api/**', async route => {
      const path = new URL(route.request().url()).pathname, method = route.request().method();
      let data = { items: [], total: 0, page: 1, page_size: 50 };
      if (path === '/api/me') data = { user: { id: '1', name: 'Owner', email: 'owner@example.invalid', is_superadmin: false }, workspaces: [{ id: '1', name: 'Demo', slug: 'demo', role_name: 'Owner', permissions: ['conversations.read', 'conversations.run', 'hosts.read', 'ai-agents.probe'] }] };
      else if (path === '/api/setup') data = { required: false };
      else if (path === '/api/profile/1') data = {id:'1',name:'Owner',email:'owner@example.invalid',email_verified:false,version:1};
      else if (path.endsWith('/workflow-node-catalog')) data = { nodes: [] };
      else if (path.endsWith('/preferences')) data = { default_page_size: 20, dictionaries: {} };
      else if (path.endsWith('/nodes/42')) data = { id: '42', name: 'Runner capability acceptance', connection: 'runner', state: 'online', os: 'linux', agent_version: '0.10.3-alpha', timezone_supported: true, timezone: '', runner_auto_update: 'off', tag_ids: [], version: 1 };
      else if (path.endsWith('/nodes/42/file-tools')) { probes++; data = { tools: ['ffmpeg capability probe'], write_workdir: true }; }
      else if (path.endsWith('/conversations')) {
        await initialList;
        // Deliberately stale: current detail must keep the selected badge accurate.
        data = { items: [{ id: '2', title: row.title, agent_name: 'Report assistant', kind: 'private', state: 'active', pending: 6 }], total: 1, page: 1, page_size: 50 };
      } else if (path.endsWith('/conversations/2')) {
        if (suspendNextDetail) {
          suspendNextDetail = false;
          detailSuspended = true;
          const stale = {...structuredClone(row), title:'Stale suspended response'};
          await new Promise(resolve => {releaseSuspended = resolve;});
          return route.fulfill({json:stale}).catch(()=>{});
        }
        await detailGate; data = structuredClone(row);
      }
      else if (path.endsWith('/events')) data = { items: [], next: 0, has_more: false };
      else if (/\/turns\/\d+\/(decisions|decision-resolutions)$/.test(path)) {
        const input = route.request().postDataJSON(); writes.push({ path, input });
        if (path.endsWith('decision-resolutions') && failDismiss) {
          failDismiss = false;
          return route.fulfill({ status: 503, json: { error: 'Temporary connection failure' } });
        }
        const turn = row.turns.find(turn => path.includes(`/turns/${turn.id}/`));
        const item = turn.decisions[input.request_id];
        if (path.endsWith('/decisions')) item.decision = input.allow;
        else item.resolved = input.action;
        data = { ok: true };
      } else if (path.endsWith('/conversations/2/files') && method === 'POST') {
        data = { file_id: 'fixture-file', name: 'notes.txt', mime: 'text/plain', size: 5 };
      } else if (method !== 'GET') data = { ok: true };
      await route.fulfill({ json: data });
    });
    await page.goto(`${origin}/en-US/workspaces/1/conversations`);
    await page.locator('main [data-slot="loading-skeleton"]').waitFor();
    assert.equal(await page.getByRole('heading', {name:'Start a conversation',exact:true}).count(), 0, 'no new-chat empty flash before the list arrives');
    releaseList();
    if(width<768){
      // Phones open the list first; a conversation is its own page without the bottom navigation.
      const list=page.locator('[data-mobile-conversation-list]');await list.waitFor();
      assert.equal(await page.locator('textarea').count(),0,'no composer on the list page');
      await list.getByRole('link',{name:new RegExp(row.title)}).first().tap();
    }
    await page.getByRole('heading', { name: row.title, exact: true }).waitFor();
    assert.equal(await page.locator('[data-pending-banner]').count(), 0);
    if(width<768){
      assert.equal(await page.locator('[data-conversation-pending-indicator]').textContent(),'6');
      await page.locator('[data-conversation-more]').tap();
      await page.locator('[data-conversation-actions]').getByRole('button',{name:'Switch workspace',exact:true}).waitFor();
    }
    await page.getByRole('button', { name: 'Needs action · 6', exact: true }).click();
    assert.equal(await page.locator('[data-ended-decision]').count(), 4);
    assert.equal(await page.locator('[data-pending-decision]').count(), 2);
    assert.equal(await page.locator('[data-pending-badge]').textContent(), '6');
    await page.screenshot({ path: join(evidence, `${width}-pending.png`), fullPage: true });
    await page.locator('[data-ended-decision="a"]').getByRole('button', { name: 'Abandon this operation' }).click();
    await page.locator('[data-ended-decision="a"] [role="alert"]').waitFor();
    await page.evaluate(() => document.documentElement.classList.add('dark'));
    await page.screenshot({ path: join(evidence, `${width}-retry-dark.png`), fullPage: true });
    await page.evaluate(() => document.documentElement.classList.remove('dark'));
    assert.equal(count(), 6, 'failed dismissal stays actionable');
    await page.locator('[data-ended-decision="a"]').getByRole('button', { name: 'Abandon this operation' }).click();
    await page.locator('[data-ended-decision="a"]').waitFor({ state: 'detached' });
    for (const key of ['b', 'c', 'd']) {
      await page.locator(`[data-ended-decision="${key}"]`).getByRole('button', { name: 'Abandon this operation' }).click();
      await page.locator(`[data-ended-decision="${key}"]`).waitFor({ state: 'detached' });
    }
    for (const [key, name] of [['e', 'Allow this request'], ['f', 'Deny this request']]) {
      await page.locator(`[data-pending-decision="${key}"]`).getByRole('button', { name, exact: true }).click();
      await page.locator(`[data-pending-decision="${key}"]`).waitFor({ state: 'detached' });
    }
    await page.waitForFunction(() => !document.querySelector('[data-pending-badge]'));
    assert.equal(count(), 0);
    if(width<768){
      assert.equal(await page.locator('[data-conversation-pending-indicator]').count(),0);
      await page.locator('[data-conversation-more]').tap();
    }
    assert.equal(await page.getByRole('button', { name: 'Needs action', exact: true }).count(), 1);
    await page.waitForTimeout(3300); // At least one stale sidebar poll must arrive.
    assert.equal(await page.locator('[data-pending-badge]').count(), 0);
    await page.reload();
    await page.getByRole('heading', { name: row.title, exact: true }).waitFor();
    await page.waitForFunction(() => !document.querySelector('[data-pending-badge]'));
    assert.equal(await page.locator('[data-ended-decision],[data-pending-decision]').count(), 0);
    assert(writes.every(write => !write.path.endsWith('/turns')), 'never sends a new reply to abandon');
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'no page overflow');
    assert.deepEqual(errors, []);
    await page.screenshot({ path: join(evidence, `${width}-settled.png`), fullPage: true });
    console.log(`PASS ${width}px: six requests, failure/retry, abandon during execution, allow/deny, stable badge, reload, no overflow or runtime errors`);
    await page.goto(`${origin}/en-US/workspaces/1/nodes/42`);
    const tools = page.getByTestId('host-file-tools');
    await tools.getByText('ffmpeg capability probe', { exact: true }).waitFor();
    assert.equal(probes, 1, 'host detail automatically probes once');
    await tools.getByRole('button').click();
    await page.waitForFunction(() => document.querySelector('[data-testid="host-file-tools"]')?.getAttribute('aria-busy') === 'false');
    assert.equal(probes, 2, 'manual refresh probes again');
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    assert.deepEqual(errors, []);
    await page.screenshot({ path: join(evidence, `${width}-host-capabilities.png`), fullPage: true });
    console.log(`PASS ${width}px: automatic host capability detection and manual refresh`);
    // Return through ordinary application links, with the thread API deliberately
    // held: previously loaded content and the composer must be ready immediately.
    await page.goto(`${origin}/en-US/workspaces/1/conversations/2`);
    await page.getByRole('heading', {name:row.title,exact:true}).waitFor();
    await page.locator('textarea').fill('Keep this unsent draft');
    await page.evaluate(() => document.documentElement.dataset.navigationToken = 'same-document');
    const go = async path => page.evaluate(path => {
      const link=document.createElement('a'); link.href=path; document.body.append(link); link.click(); link.remove();
    }, path);
    await go('/en-US/workspaces/1/nodes/42');
    await page.getByTestId('host-file-tools').waitFor();
    await go('/en-US/account');
    await page.locator('input[name=name]').waitFor();
    let releaseDetail;
    detailGate = new Promise(resolve => releaseDetail = resolve);
    await go('/en-US/workspaces/1/conversations');
    if(width<768){
      // The menu shows the list on phones; reopening the conversation restores its draft.
      const list=page.locator('[data-mobile-conversation-list]');await list.waitFor();
      await list.getByRole('link',{name:new RegExp(row.title)}).first().tap();
    }
    await page.getByRole('heading', {name:row.title,exact:true}).waitFor();
    assert.equal(await page.locator('main [data-slot="loading-skeleton"]').count(),0,'cached room has no loading replacement');
    assert.equal(await page.locator('textarea').inputValue(),'Keep this unsent draft');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.navigationToken),'same-document');
    assert(await page.evaluate(() => document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:join(evidence,`${width}-conversation-return.png`),fullPage:true});
    releaseDetail();
    assert.deepEqual(errors, []);
    console.log(`PASS ${width}px: cold entry has no empty flash, returning restores thread and unsent draft before the API responds, document retained`);
    // A suspended browser request must not hold the conversation hostage after
    // a foreground/network wake. No message or operation is replayed.
    suspendNextDetail = true;
    await page.evaluate(()=>window.dispatchEvent(new Event('pageshow')));
    for(let attempt=0;!detailSuspended && attempt<50;attempt++) await page.waitForTimeout(100);
    assert(detailSuspended,'fixture must suspend one conversation read');
    row.title = 'Conversation recovered after background';
    await page.evaluate(()=>window.dispatchEvent(new Event('online')));
    await page.getByRole('heading',{name:row.title,exact:true}).waitFor();
    releaseSuspended();
    await page.waitForTimeout(100);
    assert.equal(await page.getByRole('heading',{name:'Stale suspended response',exact:true}).count(),0);
    assert.equal(await page.locator('textarea').inputValue(),'Keep this unsent draft');
    assert.equal(await page.evaluate(()=>document.documentElement.dataset.navigationToken),'same-document');
    assert(writes.every(write=>!write.path.endsWith('/turns')),'foreground recovery never replays a message');
    assert.deepEqual(errors,[]);
    console.log(`PASS ${width}px: suspended read aborted on resume, new content loaded, late result ignored, draft/document retained`);
    if (width === 390) {
      await page.evaluate(()=>localStorage.setItem('ui.chat.composerHeight','288'));
      await go('/en-US/account');await page.locator('input[name=name]').waitFor();
      await go('/en-US/workspaces/1/conversations/2');
      await page.getByRole('heading',{name:row.title,exact:true}).waitFor();
      await page.setViewportSize({width:375,height:667});
      await page.locator('textarea').evaluate(element=>element.blur());
      await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
      const density=await page.evaluate(()=>({feed:document.querySelector('.conversation-feed').clientHeight,input:document.querySelector('textarea').getBoundingClientRect().height,composer:document.querySelector('[data-conversation-composer]').getBoundingClientRect().height,header:document.querySelector('[data-conversation-header]').getBoundingClientRect().height}));
      assert(density.feed>=667*.6,'default message region too small: '+JSON.stringify(density));
      assert(density.input<=50 && density.composer<=64 && density.header<=64,'default input/title are not single-row: '+JSON.stringify(density));
      await page.locator('[data-composer-more]').tap();
      for(const name of ['Attach files','Record voice message','Stop','Send shortcut']) await page.getByRole('menuitem',{name,exact:true}).waitFor();
      const chooser = page.waitForEvent('filechooser');
      await page.getByRole('menuitem',{name:'Attach files',exact:true}).click();
      await (await chooser).setFiles({name:'notes.txt',mimeType:'text/plain',buffer:Buffer.from('hello')});
      await page.locator('[data-composer-attachments]').getByText('notes.txt',{exact:true}).waitFor();
      await page.locator('[data-composer-attachments] button').click();
      await page.locator('[data-composer-attachments]').waitFor({state:'detached'});
      await page.locator('[data-composer-more]').tap();
      await page.keyboard.press('Escape');
      await page.evaluate(()=>{document.documentElement.dataset.nativeClient='mobile';window.dispatchEvent(new Event('client-shell:ready'));});
      await page.locator('[data-conversation-more]').tap();
      assert.equal(await page.locator('[data-conversation-actions] [data-client-connection-entry]').getAttribute('href'),'app-settings://connection');
      await page.locator('[data-conversation-actions]').getByRole('button',{name:'Search messages',exact:true}).click();
      await page.getByRole('textbox',{name:'Search messages',exact:true}).fill('report');
      assert(await page.getByRole('textbox',{name:'Search messages',exact:true}).evaluate(element=>element===document.activeElement),'search receives focus from More');
      await page.locator('[data-conversation-header]').getByRole('button',{name:'Cancel',exact:true}).click();
      await page.evaluate(()=>{delete document.documentElement.dataset.nativeClient;window.dispatchEvent(new Event('client-shell:ready'));});
      await page.screenshot({path:join(evidence,'compact-chat-default.png'),fullPage:true});
      await page.locator('textarea').fill('Long draft\n'.repeat(12));
      const grown=await page.locator('textarea').evaluate(element=>({height:element.getBoundingClientRect().height,scroll:element.scrollHeight,client:element.clientHeight}));
      assert(grown.height>density.input && grown.scroll>grown.client,'long draft must grow and then scroll');
      await page.locator('textarea').fill('Keep this unsent draft');
      console.log('PASS compact chat density: '+JSON.stringify(density)+'; plus menu and bounded draft growth');
      const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      const bounds = async (label, focused, top = 0, height = page.viewportSize().height) => {
        const metrics = await page.evaluate(() => {
          const box = selector => { const element = document.querySelector(selector), rect = element.getBoundingClientRect(); return {x:rect.x,y:rect.y,right:rect.right,bottom:rect.bottom,height:rect.height,width:rect.width,client:element.clientHeight,scroll:element.scrollHeight}; };
          return {send:box('[data-conversation-composer] button[type=submit]'),tools:box('[data-composer-tools]'),feed:box('.conversation-feed'),nav:box('[data-mobile-navigation]'),frame:box('[data-app-viewport]'),width:innerWidth,document:document.documentElement.scrollWidth,draft:document.querySelector('textarea').value};
        });
        assert(metrics.send.y >= top && metrics.send.bottom <= top+height+1,label+' send outside viewport: '+JSON.stringify(metrics));
        assert(metrics.send.x>=0 && metrics.send.right<=metrics.width+1,label+' send outside width');
        assert(metrics.send.height>=44 && metrics.send.width>=44,label+' send touch target');
        assert(metrics.tools.width>=44,label+' attachment/record controls lost their scroll area');
        assert(metrics.feed.client>8,label+' message region collapsed');
        assert(metrics.document<=metrics.width,label+' whole-page horizontal overflow');
        assert.equal(metrics.draft,'Keep this unsent draft',label+' draft lost');
        // An open conversation is its own page on phones: no bottom navigation, only the composer.
        assert(!metrics.nav || metrics.nav.height===0,label+' bottom navigation must be hidden in an open conversation');
      };
      for (const locale of ['en-US','zh-CN','ms-MY']) {
        await go(`/${locale}/workspaces/1/conversations/2`);
        for (const size of [{width:375,height:667},{width:320,height:568},{width:667,height:375},{width:375,height:360}]) {
          await page.locator('textarea').evaluate(element=>element.blur());
          await page.setViewportSize(size);
          const focused=size.height<=500;
          if (focused) await page.locator('textarea').focus();
          await settle();await bounds(`${locale}-${size.width}-${size.height}`,focused);
          const sendTop=await page.locator('[data-conversation-composer] button[type=submit]').evaluate(element=>element.getBoundingClientRect().top);
          await page.locator('.conversation-feed').evaluate(element=>element.scrollTop=0);
          assert.equal(await page.locator('[data-conversation-composer] button[type=submit]').evaluate(element=>element.getBoundingClientRect().top),sendTop,'scrolling messages must not move send controls');
          assert.equal(await page.locator('[data-platform-content]').evaluate(element=>element.scrollTop),0,'conversation must not scroll its outer shell');
          await page.screenshot({path:join(evidence,`h5-${locale}-${size.width}-${size.height}.png`),fullPage:true});
        }
      }
      // Simulate a keyboard that shrinks only visualViewport, as on iOS.
      await page.setViewportSize({width:375,height:667});
      await page.locator('textarea').focus();
      await page.evaluate(()=>{const view=visualViewport;for(const [key,value] of Object.entries({height:360,offsetTop:20}))Object.defineProperty(view,key,{configurable:true,value});view.dispatchEvent(new Event('resize'));});
      await settle();await bounds('visual-keyboard',true,20,360);
      // Back goes to the conversation list page (messaging-app navigation), which also fits the keyboard viewport.
      await page.locator('[data-conversation-navigation] button').tap();
      const list=page.locator('[data-mobile-conversation-list]');await list.waitFor();
      await list.locator('input').focus();await settle();
      const listBox=await list.boundingBox();
      assert(listBox.y>=20 && listBox.y<=381,'conversation list must start inside the keyboard viewport');
      await page.evaluate(()=>{for(const key of ['height','offsetTop'])delete visualViewport[key];visualViewport.dispatchEvent(new Event('resize'));});
      await list.locator('input').evaluate(element=>element.blur());await settle();
      assert.equal(await page.locator('[data-mobile-navigation]').isVisible(),true,'bottom navigation returns on the list page once the keyboard closes');
      await list.getByRole('link',{name:new RegExp(row.title)}).first().tap();
      await page.locator('textarea').waitFor();
      assert.equal(await page.locator('[data-mobile-navigation]').isVisible(),false,'an open conversation hides the bottom navigation');
      await page.locator('textarea').evaluate(element=>element.blur());await settle();await bounds('keyboard-dismissed',false);
      await page.emulateMedia({reducedMotion:'reduce'});
      await page.evaluate(()=>{document.documentElement.style.fontSize='32px';document.documentElement.classList.add('dark');});
      await page.locator('textarea').focus();await settle();await bounds('large-text-dark-reduced-motion',true);
      await page.screenshot({path:join(evidence,'h5-large-text-dark.png'),fullPage:true});
      await page.evaluate(()=>{document.documentElement.style.removeProperty('font-size');document.documentElement.classList.remove('dark');});
      await page.locator('textarea').evaluate(element=>element.blur());await settle();
      assert.equal(await page.evaluate(()=>localStorage.getItem('ui.chat.composerHeight')),'288','keyboard and rotation must not overwrite the resize preference');
      console.log('PASS H5: zh/en/ms phone/landscape/short viewport, visual-only keyboard resize/dismiss, 200% text, dark and reduced motion; send/navigation reachable and drafts retained');
    }
    await context.close();
  }
  console.log(`Evidence: ${evidence}`);
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
