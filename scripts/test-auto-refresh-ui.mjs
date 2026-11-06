/** Chromium acceptance for shared list and host-monitor countdowns. */
import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { preview } from 'vite';

const server = process.env.AUTO_REFRESH_UI_URL ? undefined : await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.AUTO_REFRESH_UI_URL || server.resolvedUrls.local[0].replace(/\/$/, '');
const browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
try {
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  let listReads = 0;
  let holdList = false;
  let failList = false;
  let releaseList;
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    let data = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/setup') data = { required: false, has_users: true };
    else if (path === '/api/me') data = { user: { id: '1', name: 'Reviewer' }, workspaces: [{ id: '2', name: 'Test', slug: 'test', permissions: ['tags.read', 'hosts.read', 'host-groups.read'] }] };
    else if (path.endsWith('/preferences')) data = { default_page_size: 20, dictionaries: {} };
    else if (path.endsWith('/tags')) {
      listReads++;
      if (holdList) await new Promise(resolve => { releaseList = resolve; });
    } else if (path.endsWith('/nodes/17')) data = { id: '17', name: 'Test host', connection: 'ssh', state: 'offline', tag_ids: [], version: 1 };
    else if (path.endsWith('/metrics')) data = { status: 'collecting', samples: [], server_time: Date.now() };
    const failed = path.endsWith('/tags') && failList;
    await route.fulfill({ status: failed ? 503 : 200, contentType: 'application/json', body: JSON.stringify(failed ? { error: 'Refresh failed' } : data) });
  });
  await page.goto(`${origin}/zh-CN/workspaces/2/tags`);
  const choice = page.getByRole('button', { name: '定时刷新', exact: true });
  const refreshControl = page.locator('[data-slot=refresh-control]');
  assert.equal(await choice.innerText(), '', 'disabled auto-refresh uses an icon without off text');
  assert.equal(await choice.locator('svg').count(), 1);
  const pill = await refreshControl.evaluate(element => {
    const outer = element.getBoundingClientRect();
    const buttons = [...element.querySelectorAll('button')].map(button => {
      const bounds = button.getBoundingClientRect(); const style = getComputedStyle(button);
      return { x: bounds.x, width: bounds.width, radius: parseFloat(style.borderTopLeftRadius), border: parseFloat(style.borderLeftWidth) };
    });
    return { radius: parseFloat(getComputedStyle(element).borderTopLeftRadius), height: outer.height, buttons };
  });
  assert.ok(pill.radius >= pill.height / 2, 'single capsule outline');
  assert.ok(pill.buttons[0].width > pill.buttons[1].width, 'natural unequal segment widths');
  assert.ok(pill.buttons.every(button => button.border === 0 && button.radius === 0), 'no separate button outlines or rounded inner corners');
  for (const theme of ['modern', 'business', 'eyeCare', 'pinkKawaii', 'neoBrutalism', 'custom']) {
    for (const dark of [false, true]) {
      await page.evaluate(({ theme, dark }) => {
        document.documentElement.dataset.theme = theme;
        document.documentElement.classList.toggle('dark', dark);
      }, { theme, dark });
      assert.equal(await refreshControl.evaluate(element => [...element.querySelectorAll('button')].every(button => {
        const style = getComputedStyle(button);
        return parseFloat(style.borderRadius) === 0 && parseFloat(style.borderWidth) === 0;
      })), true, `${theme}/${dark}: segments retain a single outer capsule`);
    }
  }
  await page.evaluate(() => { document.documentElement.dataset.theme = 'modern'; document.documentElement.classList.remove('dark'); });
  await choice.click();
  assert.equal(await page.getByRole('menuitem').count(), 6, 'all shared refresh choices are available');
  await page.getByRole('menuitem', { name: '每 10 秒', exact: true }).click();
  const timer = page.getByRole('timer');
  assert.equal(await timer.innerText(), '10s');
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '9s');
  const baseline = listReads;
  holdList = true;
  await page.waitForFunction(() => document.querySelector('[role=timer][aria-busy=true]')?.textContent === '0s', null, { timeout: 15000 });
  assert.equal(await timer.locator('svg').count(), 0, 'countdown does not duplicate the refresh icon');
  assert.equal(await page.getByRole('button', { name: '刷新', exact: true }).locator('svg.animate-spin').count(), 1, 'only the existing refresh icon spins');
  assert.equal(listReads, baseline + 1);
  await page.waitForTimeout(1200);
  assert.equal(listReads, baseline + 1, 'slow requests must not overlap');
  holdList = false; releaseList();
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '10s');
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '9s');
  const beforeManual = listReads;
  await page.getByRole('button', { name: '刷新', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '10s');
  assert.equal(listReads, beforeManual + 1, 'manual refresh sends exactly one request');
  holdList = true; failList = true;
  await page.getByRole('button', { name: '刷新', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[role=timer][aria-busy=true]')?.textContent === '0s');
  const failedReads = listReads;
  await page.waitForTimeout(1200);
  assert.equal(listReads, failedReads, 'manual pending request also pauses the countdown');
  holdList = false; releaseList();
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '10s');
  assert.equal(await page.getByRole('alert').count(), 1, 'request failure stays visible');
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '9s');
  failList = false;
  await choice.click(); await page.getByRole('menuitem', { name: '每 30 秒', exact: true }).click();
  assert.equal(await timer.innerText(), '30s');
  await choice.click(); await page.getByRole('menuitem', { name: '关闭自动刷新', exact: true }).click();
  assert.equal(await timer.count(), 0);
  assert.equal(await choice.innerText(), '', 'turning polling off restores the chevron');
  assert.equal(await choice.locator('svg').count(), 1);
  await page.goto(`${origin}/zh-CN/workspaces/2/nodes/17?tab=monitor`);
  await choice.click();
  await page.getByRole('menu').waitFor();
  assert.deepEqual(await page.getByRole('menuitem').allTextContents(), ['关闭自动刷新', '每 5 秒', '每 10 秒', '每 15 秒', '每 30 秒', '每 60 秒']);
  await page.getByRole('menuitem', { name: '每 5 秒', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '5s');
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '4s');
  await page.getByRole('button', { name: '刷新', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[role=timer]')?.textContent === '5s');
  assert.deepEqual(errors, []);
  console.log('PASS countdown, one existing spinner plus 0s during requests, success/failure restart, no overlap, manual reset, shared intervals/off and host-monitor countdown');
} finally { await browser.close(); if (server) await server.httpServer.close(); }
