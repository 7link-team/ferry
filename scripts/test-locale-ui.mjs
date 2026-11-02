/** Real browser checks for locale URLs, request headers, drafts and header controls. */
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
const common = JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/common-domain.json', import.meta.url), 'utf8'));
const resources = JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/resource-domain.json', import.meta.url), 'utf8'));
import { preview } from 'vite';
import { chromium } from 'playwright';
const languageTrigger = page => page.getByRole('button', { name: /^(Language|语言|語言|言語|언어|Bahasa)$/ });
const switchLanguage = async (page, locale) => {
  await languageTrigger(page).click();
  await page.locator('[role="menuitemradio"][lang="' + locale + '"]').click();
};


// Run npm run build first; verify the same output that ships.
const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = server.resolvedUrls.local[0].replace(/\/$/, '');
const installedChrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const executablePath = process.env.PLAYWRIGHT_CHROME_PATH || (existsSync(installedChrome) ? installedChrome : undefined);
const browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
const context = await browser.newContext({ locale: 'ja-JP', viewport: { width: 1366, height: 768 } });
const page = await context.newPage();
page.setDefaultTimeout(45_000);
const errors = [];
page.on('pageerror', error => errors.push(error.message));
page.on('requestfailed', request => console.error('Browser request failed:', request.url(), request.failure()?.errorText));
const requests = [];
await page.route('**/api/**', async route => {
  const request = route.request();
  const path = new URL(request.url()).pathname.slice(4);
  requests.push({ path, language: request.headers()['accept-language'] });
  let body = { items: [], total: 0, page: 1, page_size: 20 };
  if (path === '/me') body = { user: { id: '1', name: 'Operator', email: 'operator@example.invalid', is_superadmin: true }, workspaces: [{ id: '2', slug: 'ops', name: 'Operations', permissions: ['hosts.read', 'hosts.update', 'hosts.files', 'dashboard.read'], role_name: 'Owner' }] };
  if (path === '/setup') body = { required: false, has_users: true, registration_enabled: true };
  if (path.endsWith('/preferences')) body = { default_page_size: 20, dictionaries: {} };
  if (path === '/workspaces/2/nodes/42') body = { id: '42', name: 'Original host', address: 'host.example.invalid', port: 22, username: 'operator', connection: 'ssh', credential_id: '0', group_id: '0', jump_host_id: '0', tag_ids: [], tags: [], state: 'unknown', description: '', version: 1 };
  if (path === '/profile/1') body = { id: '1', name: 'Operator', email: 'operator@example.invalid', version: 1 };
  if (path === '/platform/settings/1') body = { id: '1', version: 1, runner_auto_update: false, registration_enabled: false };
  if (path.endsWith('/file-systems')) body = { platform: 'unix', items: [] };
  await route.fulfill({ json: body });
});

try {
  await page.goto(origin + '/workspaces/2/nodes/42/edit?tab=details#anchor', { waitUntil: 'commit' });
  await page.waitForURL('**/ja-JP/workspaces/2/nodes/42/edit?tab=details#anchor', { waitUntil: 'commit' });
  const name = page.locator('input[name="name"]');
  await name.fill('Unsaved local draft');
  await switchLanguage(page, 'ko-KR');
  await page.waitForURL('**/ko-KR/workspaces/2/nodes/42/edit?tab=details#anchor', { waitUntil: 'commit' });
  assert.equal(await name.inputValue(), 'Unsaved local draft');
  assert.equal(await page.locator('html').getAttribute('lang'), 'ko-KR');
  await languageTrigger(page).click();
  assert.equal(await page.getByRole('menuitemradio').count(), 6);
  await page.keyboard.press('Escape');
  assert(requests.some(request => request.path === '/me' && request.language === 'ja-JP'));
  for (const locale of Object.keys(resources)) {
    await switchLanguage(page, locale);
    await page.getByText(resources[locale]['host.port'], { exact: true }).waitFor();
    await page.getByText(resources[locale]['host.jumpHelp'], { exact: true }).waitFor();
    assert.equal(await name.inputValue(), 'Unsaved local draft');
  }
  await page.goto(origin + '/ko-KR/account', { waitUntil: 'commit' });
  await page.locator('input[name="name"]').waitFor();
  assert(requests.some(request => request.path === '/profile/1' && request.language === 'ko-KR'));

  // Actual translated profile labels, not just language options or URL changes.
  await page.locator('input[name="name"]').fill('Unsaved profile draft');
  for (const locale of ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY']) {
    await switchLanguage(page, locale);
    await page.waitForURL(`**/${locale}/account`, { waitUntil: 'commit' });
    await page.getByRole('heading', { name: common[locale]['account.title'], exact: true }).waitFor();
    await page.getByText(common[locale]['account.verifyHelp'], { exact: true }).waitFor();
    assert.equal(await page.locator('input[name="name"]').inputValue(), 'Unsaved profile draft');
    await page.getByText(common[locale]['account.loginEmail'], { exact: true }).waitFor();
  }

  await page.goto(origin + '/en-US/platform/settings/1/edit', { waitUntil: 'commit' });
  const policy = page.getByRole('checkbox').first();
  await policy.click();
  const policyLoads = requests.filter(request => request.path === '/platform/settings/1').length;
  for (const locale of Object.keys(resources)) {
    await switchLanguage(page, locale);
    await page.getByText(resources[locale]['runner.defaultAutoHelp'], { exact: true }).waitFor();
    assert.equal(await policy.getAttribute('aria-checked'), 'true');
  }
  assert.equal(requests.filter(request => request.path === '/platform/settings/1').length, policyLoads);

  // Every existing shell owns the selector in its header; no floating control covers actions.
  for (const width of [1366, 390]) {
    await page.setViewportSize({ width, height: 768 });
    for (const path of ['/en-US/account', '/zh-CN/platform/users', '/ms-MY/workspaces/2/nodes', '/ja-JP/workspaces/2/nodes/42/workspace?tool=files']) {
      await page.goto(origin + path, { waitUntil: 'commit' });
      const select = languageTrigger(page);
      await select.waitFor({ state: 'visible' });
      const box = await select.boundingBox();
      assert(box && box.x >= 0 && box.x + box.width <= width + 1, `${width}px selector outside viewport: ${path}`);
      const overlap = await select.evaluate(element => {
        const own = element.getBoundingClientRect();
        return [...element.closest('header').querySelectorAll('button, a')].some(other => {
          const rect = other.getBoundingClientRect();
          return other !== element && rect.width && rect.height && Math.min(own.right, rect.right) > Math.max(own.left, rect.left) && Math.min(own.bottom, rect.bottom) > Math.max(own.top, rect.top);
        });
      });
      assert.equal(overlap, false, `${width}px selector overlaps header action: ${path}`);
    }
  }
  assert.deepEqual(errors, []);
  console.log('PASS: six translated host/profile/platform screens, preserved host/profile drafts and platform policy, browser detection, prefixed deep links, locale headers, query/fragment preservation, and four shells at 1366px/390px.');
} catch (error) {
  console.error('Locale browser failure:', page.url(), errors);
  throw error;
} finally {
  await browser.close();
  await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
