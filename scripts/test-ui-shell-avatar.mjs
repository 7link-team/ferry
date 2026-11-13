/** Real isolated account persistence, shared shell, theme and responsive acceptance. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import { chromium } from 'playwright';
import { preview } from 'vite';

const work = await mkdtemp(join(tmpdir(), 'ui-shell-avatar-'));
const output = resolve('data/maintenance/ui-shell-avatar');
await mkdir(output, { recursive: true });
const socket = createServer();
await new Promise(r => socket.listen(0, '127.0.0.1', r));
const port = socket.address().port;
await new Promise(r => socket.close(r));
const origin = `http://127.0.0.1:${port}`;
const log = await open(join(output, 'server.log'), 'w');
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('FERRY_')));
const binary = resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${process.platform === 'win32' ? '.exe' : ''}`);
const server = spawn(binary, ['--data-dir', work, '--bind', `127.0.0.1:${port}`, '--desktop-control'], { env, windowsHide: true, stdio: ['pipe', log.fd, log.fd] });
const result = { checks: [], screenshots: [], limitations: ['No Figma or physical mobile keyboard available.'] };
const check = (value, label) => { assert(value, label); result.checks.push(label); };
let web, browser;
try {
  for (let i = 0; ; i++) {
    assert.equal(server.exitCode, null, 'server exited');
    try { if ((await fetch(origin + '/api/status')).ok) break; } catch {}
    assert(i < 150, 'server readiness');
    await new Promise(r => setTimeout(r, 200));
  }
  web = await preview({ preview: { host: '127.0.0.1', port: 0, open: false, proxy: { '/api': { target: origin, ws: true } } } });
  const ui = `http://127.0.0.1:${web.httpServer.address().port}`;
  browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ locale: 'zh-CN' });
  const api = async (path, data, method = 'GET', status) => {
    const response = await context.request.fetch(ui + '/api' + path, { method, headers: { 'X-Ferry-Request': '1' }, ...(data === undefined ? {} : { data }) });
    assert.equal(response.status(), status ?? (method === 'POST' ? 201 : 200), path + ': ' + await response.text());
    return response.status() === 204 ? undefined : response.json();
  };
  await api('/setup', { email: 'reviewer@example.invalid', name: 'Reviewer', password: 'local-acceptance-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() }, 'POST', 200);
  const workspace = await api('/workspaces', { name: 'Example project', slug: 'example-project' }, 'POST', 200);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const paths = { workspace: `/workspaces/${workspace.id}/deployments`, platform: '/platform/settings/1/edit', account: '/account' };
  const locales = JSON.parse(await readFile('src/shared/i18n/locales/identity.json', 'utf8'));
  const shot = async name => {
    const path = join(output, name + '.png');
    await page.screenshot({ path, fullPage: true }); result.screenshots.push(path);
  };
  const fits = async label => {
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), label + ' page width');
    const header = page.locator('[data-app-header]');
    await header.waitFor();
    check(await header.evaluate(el => el.scrollWidth <= el.clientWidth + 1), label + ' header width');
    check(await page.locator('h1,h2,h3').evaluateAll(nodes => nodes.every(el => ['rgba(0, 0, 0, 0)', 'transparent'].includes(getComputedStyle(el).backgroundColor))), label + ' headings have no forced color bars');
  };
  for (const palette of ['modern', 'business', 'eyeCare', 'neoBrutalism', 'custom']) {
    for (const mode of ['light', 'dark']) {
      await api('/me/preferences', { theme: { palette, mode, customLight: {}, customDark: {} }, locale: 'zh-CN' }, 'PATCH');
      for (const [name, path] of Object.entries(paths)) {
        await page.goto(ui + '/zh-CN' + path);
        await page.locator('[data-app-header]').waitFor();
        await page.waitForFunction(({ palette, mode }) => document.documentElement.dataset.theme === palette && document.documentElement.classList.contains('dark') === (mode === 'dark'), { palette, mode });
        if (name === 'account') await page.locator('[data-avatar-editor]').waitFor();
        for (const width of [1440, 900, 320]) {
          await page.setViewportSize({ width, height: 900 });
          await fits(`${palette}/${mode}/${name}/${width}`);
          if (palette === 'neoBrutalism' || palette === 'modern') await shot(`${palette}-${mode}-${name}-${width}`);
        }
        if (name === 'account') {
          check(await page.locator('h1').count() === 1, 'account has one title');
          check(await page.locator('[data-resource-heading]').count() === 0, 'account has no duplicate sticky title');
          const moved = await page.locator('[data-app-header]').evaluate(el => { const parent = el.parentElement; parent.scrollTop = 200; return el.getBoundingClientRect().top < 0; });
          check(moved, 'account header scrolls with content');
        }
      }
    }
  }
  await api('/me/preferences', { theme: { palette: 'neoBrutalism', mode: 'light', customLight: {}, customDark: {} } }, 'PATCH');
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(ui + '/zh-CN/platform/settings/1/edit');
  await page.locator('[data-app-header]').waitFor();
  const order = await page.locator('[data-app-header]').evaluate(el => [...el.querySelectorAll('button')].map(button => button.getAttribute('aria-label') || button.textContent));
  check(order.findIndex(text => text === '切换空间') < order.findIndex(text => text.includes('Reviewer')), 'platform space selection precedes account');
  await page.getByRole('button', { name: '切换空间', exact: true }).click();
  check(await page.getByRole('menuitemradio', { name: /Example project/ }).count() === 1, 'workspace menu remains usable');
  await page.keyboard.press('Escape');
  const languageButton = page.locator('[data-app-header] button').filter({ has: page.locator('svg.lucide-globe') });
  await languageButton.click();
  const selected = page.locator('.language-menu [role=menuitemradio][data-state=checked]');
  await selected.waitFor();
  check(await selected.evaluate(el => getComputedStyle(el).backgroundColor !== 'rgba(0, 0, 0, 0)'), 'language selected row has themed surface');
  check(await selected.evaluate(el => el.getBoundingClientRect().height >= 44), 'language rows meet touch height');
  await shot('language-menu');
  await page.keyboard.press('Escape');
  for (const style of ['portrait', 'adventurer', 'notionists', 'lorelei', 'pixel-art', 'pixel', 'initials', 'emoji']) {
    const avatar = { style, seed: 'acceptance-seed', ...(style === 'emoji' ? { emoji: '🦊' } : {}) };
    const profile = await api('/profile/1');
    await api('/profile/1', { name: 'Reviewer', email: 'reviewer@example.invalid', avatar, version: profile.version }, 'PATCH');
    const stored = await api('/profile/1');
    check(stored.avatar?.style === style, style + ' saved and read back');
    await page.goto(ui + '/zh-CN/account');
    await page.locator('[data-avatar-editor]').waitFor();
    check(await page.locator('[data-avatar-editor] [role=group]').first().locator('button[aria-pressed=true]').count() === 1, style + ' selected after reload');
    await page.waitForFunction(() => [...document.querySelectorAll('[data-avatar-editor] img')].every(img => img.complete && img.naturalWidth > 0));
  }
  for (const [locale, index] of [['zh-CN', 1], ['en-US', 0], ['ms-MY', 5]]) {
    await api('/me/preferences', { locale }, 'PATCH');
    await page.setViewportSize({ width: 320, height: 800 });
    await page.goto(ui + '/' + locale + '/account');
    await page.locator('[data-avatar-editor]').waitFor();
    await page.getByRole('button', { name: locales['pixel-art'][index], exact: true }).click();
    check(await page.getByRole('button', { name: locales['pixel-art'][index], exact: true }).getAttribute('aria-pressed') === 'true', locale + ' style selection works');
    await fits(locale + ' account');
    await shot(locale + '-avatar-choices');
  }
  assert.deepEqual(errors, []);
  result.result = 'PASS';
  console.log('PASS', result.checks.length, 'checks;', result.screenshots.length, 'screenshots;', output);
} finally {
  await writeFile(join(output, 'result.json'), JSON.stringify(result, null, 2));
  await browser?.close();
  if (web) await new Promise(r => web.httpServer.close(r));
  if (server.exitCode === null) { server.stdin.end(); await new Promise(r => server.once('exit', r)); }
  await log.close();
}
