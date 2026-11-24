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
    const context = await browser.newContext({ viewport: { width, height: 900 } });
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
      } else if (path.endsWith('/conversations/2')) { await detailGate; data = structuredClone(row); }
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
      } else if (method !== 'GET') data = { ok: true };
      await route.fulfill({ json: data });
    });
    await page.goto(`${origin}/en-US/workspaces/1/conversations`);
    await page.locator('main [data-slot="loading-skeleton"]').waitFor();
    assert.equal(await page.getByRole('heading', {name:'Start a conversation',exact:true}).count(), 0, 'no new-chat empty flash before the list arrives');
    releaseList();
    await page.getByRole('heading', { name: row.title, exact: true }).waitFor();
    assert.equal(await page.locator('[data-pending-banner]').count(), 0);
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
    await page.getByRole('heading', {name:row.title,exact:true}).waitFor();
    assert.equal(await page.locator('main [data-slot="loading-skeleton"]').count(),0,'cached room has no loading replacement');
    assert.equal(await page.locator('textarea').inputValue(),'Keep this unsent draft');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.navigationToken),'same-document');
    assert(await page.evaluate(() => document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:join(evidence,`${width}-conversation-return.png`),fullPage:true});
    releaseDetail();
    assert.deepEqual(errors, []);
    console.log(`PASS ${width}px: cold entry has no empty flash, returning restores thread and unsent draft before the API responds, document retained`);
    await context.close();
  }
  console.log(`Evidence: ${evidence}`);
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
