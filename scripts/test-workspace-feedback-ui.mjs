import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.FERRY_UI_ORIGIN || `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  let preferenceReads = 0, pendingNodes;
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    let json = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/me') json = { user: { id: '1', name: 'Tester', email: 'tester@example.invalid' }, workspaces: [{ id: '1', slug: 'ops', name: 'Workspace', permissions: ['hosts.read', 'pipelines.read', 'host-groups.read', 'pipeline-groups.read', 'members.read', 'members.create'] }] };
    else if (path === '/api/setup') json = { required: false };
    else if (path.endsWith('/preferences')) { if (path.startsWith('/api/workspaces/')) preferenceReads++; json = { default_page_size: 20, dictionaries: {} }; }
    else if (path.endsWith('/members/invitation')) json = { code: 'fixture-invite' };
    else if (path.endsWith('/nodes') && !pendingNodes) { pendingNodes = route; return; }
    await route.fulfill({ json });
  });
  await page.goto(`${origin}/en-US/workspaces/1/nodes`);
  const table = page.locator('table');
  await table.locator('tbody .motion-safe\\:animate-pulse').first().waitFor();
  assert.equal(await table.evaluate(el => el.closest('[aria-busy]')?.getAttribute('aria-busy')), 'true');
  await pendingNodes.fulfill({ json: { items: [], total: 0, page: 1, page_size: 20 } });
  const empty = table.getByRole('status');
  await empty.waitFor();
  const businessEmptyMessage = await empty.innerText();
  assert.ok(businessEmptyMessage.length > 0, 'resource-specific empty message is retained');
  assert.equal(await empty.getAttribute('data-empty-state'), 'empty');
  assert.equal(await empty.locator('svg[aria-hidden=true]').count(), 1);
  const filter = page.locator('form input[type=text]').first();
  await filter.fill('unmatched');
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[data-empty-state="filtered"]'));
  assert.equal(await empty.innerText(), businessEmptyMessage);
  for (const theme of ['modern', 'business', 'eyeCare', 'pinkKawaii', 'neoBrutalism', 'custom']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    assert.notEqual(await empty.locator('svg').evaluate(el => getComputedStyle(el).color), 'rgba(0, 0, 0, 0)');
  }
  await page.getByRole('link', { name: 'Workflows', exact: true }).click();
  await page.getByRole('link', { name: 'Pipeline', exact: true }).click();
  await page.waitForURL('**/pipelines');
  await page.locator('table').waitFor();
  assert.equal(preferenceReads, 1, 'ordinary navigation does not reload workspace preferences');
  assert.equal(await page.getByText('Loading workspace preferences…', { exact: true }).count(), 0);
  await page.getByRole('button', { name: 'All', exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'All', exact: true }).count(), 1);
  assert.equal(await page.getByRole('button', { name: 'Ungrouped', exact: true }).count(), 1);
  await page.setViewportSize({ width: 375, height: 800 });
  const svg = page.locator('table [role=status] svg');
  await svg.waitFor();
  const box = await svg.boundingBox();
  assert.ok(box.x >= 0 && box.x + box.width <= 375);
  assert.deepEqual(errors, []);
  console.log('PASS workspace preference reuse, native navigation, All/Ungrouped, loading skeletons, empty/filter illustrations, six themes and 375px layout');
  const catalogs = JSON.parse(await readFile(new URL('../src/shared/i18n/locales/common-domain.json', import.meta.url), 'utf8'));
  for (const [locale, messages] of Object.entries(catalogs)) {
    await page.goto(`${origin}/${locale}/workspaces/1/members`);
    const invitation = page.getByRole('region', { name: messages['invitation.title'], exact: true });
    await invitation.getByRole('button', { name: messages['invitation.view'], exact: true }).click();
    assert.equal(await invitation.locator('input').inputValue(), 'fixture-invite');
    assert.ok((await invitation.innerText()).includes(messages['invitation.help']));
    assert.equal(await invitation.getByRole('button', { name: messages['invitation.copy'], exact: true }).count(), 1);
    assert.equal(await invitation.getByRole('button', { name: messages['invitation.rotate'], exact: true }).count(), 1);
  }
  assert.deepEqual(errors, []);
  console.log('PASS invitation title, help, controls and code loading in all six languages at 375px');
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
