import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';

const url = process.env.THEME_UI_URL || 'http://127.0.0.1:5193';
const installedChrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const executablePath = process.env.PLAYWRIGHT_CHROME_PATH || (process.platform === 'win32' && existsSync(installedChrome) ? installedChrome : undefined);
const browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
try {
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1366, height: 900 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/**', route => {
    const path = new URL(route.request().url()).pathname;
    let value = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/me') value = { user: { id: '1', name: 'Theme reviewer', email: 'reviewer@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions: ['hosts.read'] }] };
    if (path === '/api/setup') value = { required: false, has_users: true };
    if (path.endsWith('/preferences')) value = { default_page_size: 20, dictionaries: {} };
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
  });
  await page.addInitScript(() => { if (!localStorage.getItem('ferry.theme')) localStorage.setItem('ferry.theme', JSON.stringify({ palette: 'erp', mode: 'dark' })); });
  await page.goto(`${url}/workspaces/2/nodes`);
  const themeButton = page.getByRole('button', { name: '主题', exact: true });
  await themeButton.waitFor();
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'modern');
  assert.equal(await page.locator('html').evaluate(node => node.classList.contains('dark')), true);
  const palettes = [
    { value: 'modern', label: '现代简约' }, { value: 'business', label: '商务渐变' },
    { value: 'eyeCare', label: '护眼模式' }, { value: 'pinkKawaii', label: '粉色可爱' },
    { value: 'neoBrutalism', label: '新粗野主义' }, { value: 'custom', label: '自定义' },
  ];
  const source = readFileSync(new URL('../src/shared/theme/themes.css', import.meta.url), 'utf8');
  const choose = async name => { await themeButton.click(); await page.getByRole('menuitem', { name, exact: false }).click(); };
  for (const palette of palettes) {
    await choose(palette.label);
    if (palette.value === 'custom') await page.getByRole('button', { name: '关闭', exact: true }).click();
    for (const mode of ['light', 'dark']) {
      await choose(mode === 'light' ? '浅色' : '深色');
      const selector = `[data-theme="${palette.value}"]${mode === 'dark' ? '.dark' : ''} {`;
      const block = source.split(selector)[1].split('}')[0];
      const expected = /--background:\s*([^;]+);/.exec(block)[1];
      await page.waitForFunction(expected => getComputedStyle(document.documentElement).getPropertyValue('--background').trim() === expected, expected);
    }
  }
  await themeButton.click();
  assert.equal(await page.getByRole('menuitem', { name: /ERP/i }).count(), 0);
  await page.keyboard.press('Escape');
  await choose('自定义');
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('tab', { name: '浅色', exact: true }).click();
  await dialog.locator('input[type="text"][aria-label="主色"]').fill('#123456');
  await dialog.getByRole('button', { name: '关闭', exact: true }).click();
  await choose('浅色');
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#123456');
  await page.reload(); await themeButton.waitFor();
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#123456');
  await choose('自定义');
  await dialog.getByRole('button', { name: '导入', exact: true }).click();
  await dialog.getByRole('textbox', { name: '将导出的 JSON 粘贴到此处' }).fill('{bad');
  await dialog.getByRole('button', { name: '应用导入' }).click();
  await dialog.getByRole('status').filter({ hasText: '导入失败' }).waitFor();
  await dialog.getByRole('textbox', { name: '将导出的 JSON 粘贴到此处' }).fill(JSON.stringify({ light: { '--primary': '#654321', '--success': '#abcdef' } }));
  await dialog.getByRole('button', { name: '应用导入' }).click();
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#654321');
  await dialog.getByRole('button', { name: '随机配色' }).click();
  assert.notEqual(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#654321');
  await dialog.getByRole('button', { name: '重置', exact: true }).click();
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '');
  await page.setViewportSize({ width: 390, height: 844 });
  const bounds = await dialog.boundingBox();
  assert(bounds.x >= 0 && bounds.x + bounds.width <= 390 && bounds.height <= 844);
  await dialog.evaluate(node => { node.scrollTop = 0; });
  mkdirSync('data/maintenance', { recursive: true });
  await page.screenshot({ path: 'data/maintenance/theme-mobile.png', fullPage: true });
  const translations = JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/theme.json', import.meta.url), 'utf8'));
  await page.setViewportSize({ width: 1366, height: 900 });
  for (const [locale, catalog] of Object.entries(translations)) {
    await page.goto(`${url}/${locale}/workspaces/2/nodes`);
    const localizedTheme = page.getByRole('button', { name: catalog.label, exact: true });
    await localizedTheme.click();
    for (const palette of palettes) await page.getByRole('menuitem', { name: catalog[palette.value], exact: false }).waitFor();
    await page.getByRole('menuitem', { name: catalog.custom, exact: false }).click();
    const localizedDialog = page.getByRole('dialog');
    await localizedDialog.getByRole('heading', { name: catalog.customize, exact: true }).waitFor();
    await localizedDialog.getByLabel(catalog['customPanel.colors.primary'], { exact: true }).first().waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'), locale);
    await localizedDialog.getByRole('button', { name: catalog.close, exact: true }).click();
  }
  assert.deepEqual(errors, []);
  console.log('PASS: 6 palettes x light/dark, ERP migration/removal, custom editing/persistence/import/random/reset, mobile dialog, all six translated theme dialogs, no page errors');
} finally { await browser.close(); }
