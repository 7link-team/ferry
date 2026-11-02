/** Chromium interaction acceptance; API fixtures isolate editor behavior from backend tests. */
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';

const origin = process.env.TYPED_VALUES_UI_URL || 'http://127.0.0.1:5193';
const installedChrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const executablePath = process.env.PLAYWRIGHT_CHROME_PATH || (process.platform === 'win32' && existsSync(installedChrome) ? installedChrome : undefined);
const browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
try {
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1366, height: 900 } });
  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  page.setDefaultNavigationTimeout(60000);
  const errors = []; const requests = []; const rows = { dictionaries: [], settings: [] };
  page.on('pageerror', error => errors.push(error.message));
  const permissions = ['settings', 'dictionaries'].flatMap(resource => ['read', 'create', 'update', 'delete'].map(action => `${resource}.${action}`));
  const image = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', 'base64');
  let uploaded = 0;
  await page.route('**/api/**', async route => {
    const request = route.request(); const url = new URL(request.url()); const path = url.pathname;
    let value = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/me') value = { user: { id: '1', name: 'Reviewer', email: 'reviewer@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions }] };
    if (path === '/api/setup') value = { required: false, has_users: true };
    if (path.endsWith('/preferences')) value = { default_page_size: 20, dictionaries: {} };
    if (path.includes('/value-files') && request.method() === 'POST') {
      assert(['settings', 'dictionaries'].includes(url.searchParams.get('resource')));
      assert.match(request.headers()['content-type'], /multipart\/form-data/);
      assert(request.postDataBuffer().length > image.length);
      const id = String(++uploaded); value = { id, url: `/api/workspaces/2/value-files/${id}`, name: 'sample.png', size: image.length, content_type: 'image/png' };
    } else if (path.includes('/value-files/')) return route.fulfill({ contentType: 'application/octet-stream', body: image, headers: { 'Content-Disposition': 'attachment', 'X-Content-Type-Options': 'nosniff' } });
    const match = /^\/api\/workspaces\/2\/(dictionaries|settings)(?:\/([^/]+))?$/.exec(path);
    if (match) {
      const [, resource, id] = match;
      if (request.method() === 'POST' || request.method() === 'PATCH') {
        const body = request.postDataJSON(); requests.push({ resource, body });
        const row = { ...body, id: id || String(rows[resource].length + 1), version: Number(body.version || 0) + 1, builtin: false, value_summary: body.value_json };
        if (id) rows[resource] = rows[resource].map(old => old.id === id ? row : old); else rows[resource].push(row);
        value = { id: row.id, version: row.version };
      } else if (id) value = rows[resource].find(row => row.id === id);
      else { const filtered = rows[resource].filter(row => (!url.searchParams.get('category') || row.category === url.searchParams.get('category')) && (!url.searchParams.get('value_type') || row.value_type === url.searchParams.get('value_type'))); value = { items: filtered, total: filtered.length, page: 1, page_size: 20 }; }
    }
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
  });
  const types = ['text', 'num', 'tab', 'selection', 'image', 'video', 'file', 'code', 'markdown', 'html', 'richtext', 'json'];
  for (const type of types) {
    console.log(`Checking ${type}`);
    await page.goto(`${origin}/workspaces/2/dictionaries/new`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('textbox', { name: /^标识/ }).fill(`test-${type}`);
    await page.getByRole('textbox', { name: /^名称/ }).fill(`Typed ${type}`);
    await page.getByLabel('值类型', { exact: true }).selectOption(type);
    if (type === 'tab' || type === 'selection') {
      await page.getByRole('button', { name: '添加选项', exact: true }).click();
      await page.getByLabel('选项 1 值', { exact: true }).fill('first');
      await page.getByLabel('选项 1 显示名称', { exact: true }).fill('First');
    } else if (type === 'num') {
      await page.getByLabel('最小值', { exact: true }).fill('0');
      await page.getByLabel('最大值', { exact: true }).fill('10');
      await page.locator('input[aria-label="\u503c"],textarea[aria-label="\u503c"]').fill('3');
    } else if (['image', 'video', 'file'].includes(type)) {
      const mimeType = type === 'video' ? 'video/mp4' : 'image/png';
      await page.getByLabel('选择附件', { exact: true }).setInputFiles({ name: type === 'video' ? 'sample.mp4' : 'sample.png', mimeType, buffer: image });
      await page.getByRole('link', { name: 'sample.png', exact: true }).waitFor();
      if (type === 'image') await page.waitForFunction(() => !!document.querySelector('img[alt="sample.png"]')?.complete, undefined, { timeout: 20000 });
    } else if (type === 'code') {
      await page.getByLabel('代码值', { exact: true }).fill('const value = 1;');
      await page.getByRole('button', { name: '使用代码编辑器', exact: true }).click();
      await page.locator('.monaco-editor').waitFor();
    }
    else if (type === 'json') {
      await page.getByLabel('JSON 值', { exact: true }).fill('{bad');
      await page.getByRole('alert').filter({ hasText: 'JSON' }).waitFor();
      assert(await page.getByRole('button', { name: '创建', exact: true }).isDisabled());
      await page.getByLabel('JSON 值', { exact: true }).fill('{"enabled":true}');
    } else if (type === 'richtext') {
      await page.getByRole('textbox', { name: '富文本值', exact: true }).click();
      await page.getByRole('textbox', { name: '富文本值', exact: true }).pressSequentially('Rich content');
      await page.locator('[data-slate-editor]').press('ControlOrMeta+A');
      await page.waitForFunction(() => !document.querySelector('.w-e-toolbar [data-menu-key="bold"]')?.classList.contains('disabled'));
      await page.locator('.w-e-toolbar [data-menu-key="bold"]').click();
      await page.waitForFunction(() => [...document.querySelectorAll('[data-slate-string]')].some(element => Number(getComputedStyle(element).fontWeight) >= 600));
      await page.locator('[data-slate-editor]').evaluate(element => { window.__richEditorNode = element; });
      const count = requests.length;
      for (const [locale, bold] of [['en-US', 'Bold'], ['zh-TW', '粗體'], ['ja-JP', '太字'], ['ko-KR', '굵게'], ['ms-MY', 'Tebal'], ['zh-CN', '粗体']]) {
        await page.locator('header select').first().selectOption(locale);
        await page.waitForFunction(expected => document.querySelector('.w-e-toolbar [data-menu-key="bold"]')?.getAttribute('data-tooltip')?.split('\n')[0] === expected, bold);
        assert.equal(await page.locator('[data-slate-editor]').innerText(), 'Rich content');
        assert(await page.locator('[data-slate-editor]').evaluate(element => element === window.__richEditorNode));
        assert.equal(requests.length, count);
      }
      await page.locator('[data-slate-editor]').focus();
      await page.locator('[data-slate-editor]').press('ControlOrMeta+Z');
      await page.waitForFunction(() => [...document.querySelectorAll('[data-slate-string]')].every(element => Number(getComputedStyle(element).fontWeight) < 600));
      assert.equal(await page.locator('[data-slate-editor]').innerText(), 'Rich content');
      mkdirSync('data/maintenance/typed-values-ui', { recursive: true });
      await page.screenshot({ path: 'data/maintenance/typed-values-ui/richtext-desktop.png', fullPage: true });
    }
    else await page.locator('input[aria-label="\u503c"],textarea[aria-label="\u503c"]').fill(type === 'markdown' ? '# Heading\n\n| A | B |\n|---|---|\n| 1 | 2 |' : type === 'html' ? '<h1>HTML preview</h1><script>parent.window.__unexpected = true</script>' : 'Example text');
    await page.getByRole('button', { name: '创建', exact: true }).click();
    await page.getByRole('cell', { name: `test-${type}`, exact: true }).waitFor();
    const saved = requests.at(-1).body;
    assert.equal(saved.value_type, type); assert.equal(saved.category, 'general'); JSON.parse(saved.value_json); JSON.parse(saved.schema_json);
    const row = rows.dictionaries.at(-1);
    await page.goto(`${origin}/workspaces/2/dictionaries/${row.id}`, { waitUntil: 'domcontentloaded' });
    if (type === 'markdown') await page.getByRole('heading', { name: 'Heading', exact: true }).waitFor();
    if (type === 'html') { assert.equal(await page.locator('iframe[title="内容预览"]').getAttribute('sandbox'), ''); assert.equal(await page.evaluate(() => window.__unexpected), undefined); }
  }
  const rich = rows.dictionaries.find(row => row.value_type === 'richtext');
  rich.value_json = JSON.stringify('<p><a href="https://example.invalid">Preserved link</a></p><table><tbody><tr><td>Preserved cell</td></tr></tbody></table>');
  await page.goto(`${origin}/workspaces/2/dictionaries/${rich.id}`, { waitUntil: 'domcontentloaded' });
  await page.locator('.w-e-text-container a').filter({ hasText: 'Preserved link' }).waitFor();
  await page.locator('.w-e-text-container td').filter({ hasText: 'Preserved cell' }).waitFor();
  await page.goto(`${origin}/workspaces/2/settings/new`, { waitUntil: 'domcontentloaded' });
  await page.getByRole('textbox', { name: /^标识/ }).fill('custom-setting');
  await page.getByRole('textbox', { name: /^名称/ }).fill('Custom setting');
  await page.locator('input[aria-label="\u503c"],textarea[aria-label="\u503c"]').fill('custom value');
  await page.getByRole('button', { name: '创建', exact: true }).click();
  await page.getByRole('cell', { name: 'custom-setting', exact: true }).waitFor();
  assert.equal(rows.settings[0].value_json, '"custom value"');
  await page.goto(`${origin}/workspaces/2/dictionaries`, { waitUntil: 'domcontentloaded' });
  await page.getByRole('combobox', { name: '值类型', exact: true }).click();
  await page.getByRole('option', { name: 'JSON', exact: true }).click();
  await page.getByRole('button', { name: '搜索', exact: true }).click();
  await page.waitForURL(/value_type=json/);
  assert.equal(await page.getByRole('cell', { name: 'test-json', exact: true }).count(), 1);
  assert.equal(await page.getByRole('cell', { name: 'test-html', exact: true }).count(), 0);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${origin}/workspaces/2/dictionaries/new`, { waitUntil: 'domcontentloaded' });
  await page.getByLabel('值类型', { exact: true }).selectOption('selection');
  await page.getByRole('button', { name: '添加选项', exact: true }).click();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
  await page.screenshot({ path: 'data/maintenance/typed-values-ui/options-mobile.png', fullPage: true });
  // Switch the mounted editor through all locales; user values and validation must survive.
  await page.setViewportSize({ width: 1366, height: 900 });
  await page.goto(`${origin}/zh-CN/workspaces/2/dictionaries/new`, { waitUntil: 'domcontentloaded' });
  await page.getByRole('textbox', { name: /^标识/ }).fill('locale-draft');
  await page.getByRole('textbox', { name: /^名称/ }).fill('User-owned name');
  await page.getByLabel('值类型', { exact: true }).selectOption('selection');
  await page.getByRole('button', { name: '添加选项', exact: true }).click();
  await page.getByLabel('选项 1 值', { exact: true }).fill('raw-value');
  await page.getByLabel('选项 1 显示名称', { exact: true }).fill('User-owned label');
  await page.getByRole('button', { name: '添加选项', exact: true }).click();
  const typed = JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/typed-domain.json', import.meta.url), 'utf8'));
  const locales = ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'];
  for (const [index, locale] of locales.entries()) {
    await page.locator('select').filter({ has: page.locator('option[value="en-US"]') }).selectOption(locale);
    await page.waitForURL(url => url.pathname.startsWith(`/${locale}/`));
    const label = typed.optionLabel[index].replace('{{index}}', '1');
    assert.equal(await page.getByLabel(label, { exact: true }).inputValue(), 'User-owned label');
    const valueLabel = typed.optionValue[index].replace('{{index}}', '1');
    assert.equal(await page.getByLabel(valueLabel, { exact: true }).inputValue(), 'raw-value');
    await page.getByRole('alert').filter({ hasText: typed.optionRequired[index] }).waitFor();
    assert.equal(await page.getByLabel(typed.valueType[index], { exact: true }).inputValue(), 'selection');
    assert.equal(await page.getByLabel(label, { exact: true }).count(), 1);
    assert.equal(await page.getByRole('button', { name: typed.addOption[index], exact: true }).count(), 1);
  }
  assert.deepEqual(errors, []);
  console.log('PASS: all 12 value editors create/read, typed custom settings, JSON failure and recovery, zero numeric bounds, real multipart uploads + raster blob preview, isolated HTML, CommonMark/GFM preview, filtering, mobile width, no page errors');
} finally { await browser.close(); }
