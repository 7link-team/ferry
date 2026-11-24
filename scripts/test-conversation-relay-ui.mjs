/** Production UI acceptance for multi-developer rooms with stateful API fixtures; no real Agent runs. */
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { preview } from 'vite';
import { chromium } from 'playwright';

const evidence = await mkdtemp(join(tmpdir(), 'conversation-relay-'));
const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.FERRY_UI_ORIGIN || `http://127.0.0.1:${server.httpServer.address().port}`;
const labels = {
  'en-US': { relay: 'Agent relay', held: 'Agent relays are paused', cont: 'Continue', sponsor: 'Added by', add: 'Add my Agent', members: 'Conversation members' },
  'zh-CN': { relay: 'Agent 传话', held: 'Agent 之间的传话已暂停', cont: '继续', sponsor: '添加者', add: '添加我的 Agent', members: '会话成员' },
  'ms-MY': { relay: 'Geganti Agent', held: 'Geganti antara Agent dijeda', cont: 'Teruskan', sponsor: 'Ditambah oleh', add: 'Tambah Agent saya', members: 'Ahli perbualan' },
};
let browser;
try {
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  for (const [locale, text] of Object.entries(labels)) for (const width of locale === 'en-US' ? [1440, 900, 390, 320] : [390]) {
    const context = await browser.newContext({ viewport: { width, height: 900 }, hasTouch: width < 1200, isMobile: width < 768, colorScheme: width === 320 ? 'dark' : 'light' });
    const page = await context.newPage(), errors = [], writes = [];
    page.on('pageerror', error => errors.push(error.message));
    const agent = (row, sponsor, name) => ({ id: '7', room_agent_id: row, added_by: sponsor, sponsor_name: name, approval_mode: 'request', name: 'Codex', state: 'active', runtime: 'codex', model: '', host: { id: '42', name: `host-${name}` } });
    const turn = (id, row, state, prompt) => ({ id, owner_id: row === '101' ? '1' : '2', agent_id: '7', room_agent_id: row, hop: 1, state, prompt, attachments: [], error: '', usage: null, decisions: {}, message_id: '501' });
    const row = {
      id: '2', title: 'API and app integration', kind: 'group', state: 'active', viewer_id: '1', owner_id: '1', can_manage: true, agent_id: '7',
      members: [{ id: '1', name: 'Ana' }, { id: '2', name: 'Bo' }], agents: [agent('101', '1', 'Ana'), agent('102', '2', 'Bo')],
      messages: [
        { id: '500', role: 'user', turn_id: null, reply_to: null, agent_id: '0', sender_name: 'Ana', owner_id: '1', prompt: 'Build GET /orders and the list screen', attachments: [], targets: ['7'], created_at: 1764000000000 },
        { id: '501', role: 'relay', turn_id: null, reply_to: null, agent_id: '7', sender_name: 'Codex', owner_id: '1', prompt: 'GET /orders now returns {items,total}; please update the app.', attachments: [], targets: ['7'], created_at: 1764000060000 },
      ],
      version: 1, revision_id: '1', members_version: '1', config: { name: 'Codex', runtime: 'codex', model: '', cwd: '/tmp', relay_enabled: true }, binding: null,
      turns: [turn('601', '102', 'held', 'GET /orders now returns {items,total}; please update the app.')],
    };
    await page.route('**/api/**', async route => {
      const path = new URL(route.request().url()).pathname, method = route.request().method();
      let data = { items: [], total: 0, page: 1, page_size: 50 };
      if (path === '/api/me') data = { user: { id: '1', name: 'Ana', email: 'ana@example.invalid', is_superadmin: false }, workspaces: [{ id: '1', name: 'Demo', slug: 'demo', role_name: 'Owner', permissions: ['conversations.read', 'conversations.run', 'conversations.update', 'ai-agents.read', 'ai-agents.use'] }] };
      else if (path === '/api/setup') data = { required: false };
      else if (path === '/api/profile/1') data = { id: '1', name: 'Ana', email: 'ana@example.invalid', email_verified: false, version: 1 };
      else if (path.endsWith('/preferences')) data = { default_page_size: 20, dictionaries: {} };
      else if (path.endsWith('/ai-agents')) data = { items: [{ id: '7', name: 'Codex', state: 'enabled', visibility: 'workspace' }, { id: '8', name: 'Claude Code', state: 'enabled', visibility: 'workspace' }], total: 2, page: 1, page_size: 100 };
      else if (path.endsWith('/conversations')) data = { items: [{ id: '2', title: row.title, agent_name: 'Codex', kind: 'group', state: 'active', pending: 0 }], total: 1, page: 1, page_size: 50 };
      else if (path.endsWith('/conversations/2/relay/continue') && method === 'POST') {
        writes.push(path); row.turns = row.turns.map(item => ({ ...item, state: item.state === 'held' ? 'queued' : item.state, hop: 0 })); data = { queued: 1 };
      } else if (path.endsWith('/conversations/2/relay') && method === 'PATCH') {
        const input = route.request().postDataJSON(); writes.push(`${path}:${input.enabled}`); row.config.relay_enabled = input.enabled; data = { relay_enabled: input.enabled };
      } else if (path.endsWith('/conversations/2/agents') && method === 'POST') {
        writes.push(`${path}:${route.request().postDataJSON().agent_id}`); data = { id: '103' };
      } else if (/\/conversations\/2\/agents\/\d+$/.test(path) && method === 'PATCH') {
        const input = route.request().postDataJSON(); writes.push(`${path}:${input.approval_mode}`);
        row.agents = row.agents.map(item => path.endsWith(`/${item.room_agent_id}`) ? { ...item, approval_mode: input.approval_mode } : item); data = {};
      } else if (path.endsWith('/conversations/2')) data = structuredClone(row);
      else if (path.endsWith('/events')) data = { items: [], next: 0, has_more: false };
      return route.fulfill({ json: data });
    });
    await page.goto(`${origin}/${locale}/workspaces/1/conversations/2`);
    await page.getByText('GET /orders now returns', { exact: false }).first().waitFor();
    // The relay is labelled and the held delivery can be continued.
    assert.equal(await page.locator('[data-relay-message]').count(), 1, 'relay label');
    assert((await page.locator('[data-relay-message]').innerText()).includes(text.relay));
    const held = page.locator('[data-held-relays]');
    await held.waitFor();
    assert((await held.innerText()).includes(text.held));
    await page.screenshot({ path: join(evidence, `${locale}-${width}-held.png`) });
    await held.getByRole('button', { name: text.cont, exact: true }).click();
    await held.waitFor({ state: 'detached' });
    assert(writes.some(item => item.endsWith('/relay/continue')), 'continue posted');
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'no page overflow with relay');
    // Members panel: sponsors are visible, own Agent's approval is editable, room controls work.
    await page.locator('[data-conversation-header] button').filter({ hasText: row.title }).first().click();
    const panel = page.getByRole('dialog');
    await panel.getByText('Codex (Ana)', { exact: true }).waitFor();
    await panel.getByText('Codex (Bo)', { exact: true }).waitFor();
    await panel.locator('[data-room-controls] select').selectOption('8');
    await panel.locator('[data-room-controls]').getByRole('button', { name: text.add, exact: true }).click();
    await page.waitForFunction(() => true);
    assert(writes.some(item => item.endsWith('/agents:8')), 'adds own Agent');
    await panel.locator('[data-room-controls] input[type="checkbox"]').click();
    assert(writes.some(item => item.endsWith('/relay:false')), 'relay switch');
    await panel.getByText('Codex (Ana)', { exact: true }).click();
    assert((await panel.innerText()).includes(text.sponsor));
    await panel.locator('#room-agent-approval').selectOption('assisted');
    await page.waitForFunction(() => true);
    assert(writes.some(item => item.endsWith('/agents/101:assisted')), 'sponsor changes own Agent mode');
    await page.screenshot({ path: join(evidence, `${locale}-${width}-member.png`) });
    assert(await panel.evaluate(node => node.scrollWidth <= node.clientWidth + 1), 'members panel does not overflow');
    // Another sponsor's Agent is read-only for the viewer.
    await panel.getByRole('button').first().click();
    await panel.getByText('Codex (Bo)', { exact: true }).click();
    assert.equal(await panel.locator('#room-agent-approval').count(), 0, 'cannot change another sponsor\'s mode');
    assert.deepEqual(errors, [], `${locale} ${width}: ${errors.join('; ')}`);
    await context.close();
    console.log(`PASS relay room ${locale} ${width}px`);
  }
  console.log(`evidence ${evidence}`);
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
