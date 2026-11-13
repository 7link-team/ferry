/** Inspect the real market entry through the built UI; never import or deploy it. */
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { chromium } from 'playwright';
import { preview } from 'vite';

const backend = process.env.PREVIEW_BACKEND || 'http://127.0.0.1:4291';
const output = 'data/maintenance/market-form-preview';
await mkdir(output, { recursive: true });
const web = await preview({ preview: { host: '127.0.0.1', port: 0, open: false, proxy: { '/api': { target: backend, ws: true } } } });
const browser = await chromium.launch();
const checks = [];
const translations = JSON.parse(await readFile('src/shared/i18n/locales/host-repository-domain.json', 'utf8'));
try {
  const context = await browser.newContext({ storageState: 'data/maintenance/agent-platform-tools/admin-auth-private.json' });
  const origin = `http://127.0.0.1:${web.httpServer.address().port}`;
  const path = '/api/workspaces/110015972022161408/marketplace/114501885213282305';
  const before = await context.request.get(backend + path);
  assert.equal(before.status(), 200);
  const original = await before.json();
  const page = await context.newPage();
  const errors = [], writes = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (request.url().includes('/api/workspaces/') && request.method() !== 'GET') writes.push(request.method() + ' ' + request.url()); });
  for (const locale of ['zh-CN', 'en-US', 'ms-MY']) {
    for (const width of [1440, 900, 320]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto(origin + '/' + locale + '/workspaces/110015972022161408/marketplace/114501885213282305?at=1165');
      const form = page.locator('[data-blueprint-form]');
      await form.waitFor();
      assert.equal(await page.getByRole('alert').filter({ hasText: /services/ }).count(), 0);
      assert((await form.locator('input').evaluateAll(inputs => inputs.map(input => input.value))).includes('nats:2.10-alpine'));
      assert(await form.locator('input,textarea').evaluateAll(inputs => inputs.every(input => input.readOnly)));
      const toggle = form.locator('..').getByRole('button');
      assert.equal(await toggle.count(), 1);
      await toggle.click();
      await form.waitFor({ state: 'detached' });
      await page.locator('.monaco-editor').first().waitFor();
      await page.getByRole('button', { name: translations[locale]['manifest.formPreview'], exact: true }).click();
      await form.waitFor();
      const parameterPreview = page.locator('section').filter({ has: page.locator('input[name="client_port"]') }).last();
      const number = page.locator('input[name="client_port"]');
      await number.waitFor();
      assert.equal(await number.inputValue(), '14222');
      await number.fill('14223');
      assert.equal(await number.inputValue(), '14223');
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
      await form.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `${output}/${locale}-${width}-blueprint.png` });
      await parameterPreview.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `${output}/${locale}-${width}-parameters.png` });
      const file = page.locator('details').filter({ has: page.locator('summary', { hasText: 'nats.conf' }) });
      await file.locator('summary').click();
      await file.locator('.monaco-editor').waitFor();
      assert((await file.textContent()).includes('/etc/nats/nats.conf'));
      assert(original.package_files?.some(item => item.name === 'nats.conf' && item.content.length > 0));
      await file.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `${output}/${locale}-${width}-config-file.png` });
      checks.push(`${locale}/${width}: blueprint form, one toggle roundtrip, readonly definition, transient typed preview, width`);
    }
  }
  assert.deepEqual(errors, []);
  assert.deepEqual(writes, []);
  const after = await context.request.get(backend + path);
  const final = await after.json();
  assert.equal(final.content, original.content);
  assert.equal(final.parameters, original.parameters);
  await writeFile(`${output}/result.json`, JSON.stringify({ result: 'PASS', checks, writes, errors }, null, 2));
  console.log('PASS', checks.length, 'locale/viewport scenarios; no writes or runtime errors');
} finally {
  await browser.close();
  await new Promise(resolve => web.httpServer.close(resolve));
}
