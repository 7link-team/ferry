/** Production browser regression: local editor loading, warm opens and draft retention. */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { preview } from 'vite';
import { chromium } from 'playwright';
const languageTrigger = page => page.getByRole('button', { name: /^(Language|语言|語言|言語|언어|Bahasa)$/ });
const switchLanguage = async (page, locale) => {
  await languageTrigger(page).click();
  await page.locator('[role="menuitemradio"][lang="' + locale + '"]').click();
};


const server = process.env.YAML_EDITOR_UI_URL ? null : await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.YAML_EDITOR_UI_URL || server.resolvedUrls.local[0].replace(/\/$/, '');
const chrome = process.env.PLAYWRIGHT_CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const browser = await chromium.launch({ headless: true, ...(existsSync(chrome) ? { executablePath: chrome } : {}) });
const page = await browser.newPage({ viewport: { width: 1366, height: 900 }, permissions: ['clipboard-read', 'clipboard-write'] });
page.setDefaultTimeout(20000);
const errors = [], external = [], workers = [], requests = [];
page.on('pageerror', error => errors.push(error.message));
page.on('worker', worker => workers.push(worker.url()));
page.on('request', request => {
  requests.push(request.url());
  if (!request.url().startsWith(origin) && !request.url().startsWith('data:')) external.push(request.url());
});
const definition = 'name: User workflow\non: workflow_dispatch\njobs:\n  build:\n    name: User job\n    runs-on: self-hosted\n    steps:\n      - run: echo user\n';
let loadedAt;
await page.route('**/api/**', async route => {
  const path = new URL(route.request().url()).pathname.slice(4);
  let body = { items: [], total: 0, page: 1, page_size: 200 };
  if (path === '/me') body = { user: { id: '1', name: 'Operator', email: 'operator@example.invalid', is_superadmin: true }, workspaces: [{ id: '2', name: 'Operations', slug: 'ops', permissions: ['pipelines.read', 'pipelines.update', 'pipelines.run', 'hosts.read', 'host-groups.read'], role_name: 'Owner' }] };
  if (path === '/setup') body = { required: false, has_users: true, registration_enabled: true };
  if (path.endsWith('/preferences')) body = { default_page_size: 20, dictionaries: {} };
  if (path === '/workspaces/2/pipelines/42') {
    body = { id: '42', name: 'User pipeline', description: '', version: 1, definition_format: 'github-actions', definition, executor_bindings: [], checkout_bindings: [], graph_layout: {} };
    loadedAt = performance.now();
  }
  await route.fulfill({ json: body });
});

try {
  await page.goto(origin + '/en-US/workspaces/2/pipelines/42/edit');
  const yamlTab = page.getByRole('tab', { name: 'YAML', exact: true });
  await yamlTab.waitFor();
  const timings = [];
  for (let i = 0; i < 3; i++) {
    const start = performance.now();
    await yamlTab.click();
    await page.locator('.monaco-editor .view-lines').waitFor();
    await page.locator('.monaco-editor textarea').focus();
    timings.push(Math.round(performance.now() - start));
    if (i === 0) console.log(`API response to first editor ready: ${Math.round(performance.now() - loadedAt)}ms`);
    // Unmount/remount the actual editor rather than hiding its DOM.
    await page.getByRole('tab').filter({ hasNotText: 'YAML' }).first().click();
    await page.locator('.monaco-editor').waitFor({ state: 'detached' });
  }
  await yamlTab.click();
  await page.locator('.monaco-editor .view-lines').click();
  await page.keyboard.press('ControlOrMeta+Home');
  await page.evaluate(() => navigator.clipboard.writeText('# unsaved locale draft\n'));
  await page.keyboard.press('ControlOrMeta+V');
  for (const locale of ['zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY', 'en-US']) {
    await switchLanguage(page, locale);
    await page.waitForFunction(() => document.querySelector('.monaco-editor .view-lines')?.textContent.replaceAll('\u00a0', ' ').includes('unsaved locale draft'));
  }
  await page.locator('.monaco-editor .view-lines').click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.evaluate(() => navigator.clipboard.writeText('jobs: ['));
  await page.keyboard.press('ControlOrMeta+V');
  await page.waitForFunction(() => !!document.querySelector('[class*="squiggly"]'), {}, { timeout: 15000 }).catch(async error => {
    console.log({ errors, workers, text: await page.locator('.monaco-editor .view-lines').textContent() });
    throw error;
  });
  await page.locator('.monaco-editor .view-lines').click();
  await page.keyboard.press('ControlOrMeta+A');
  await page.evaluate(() => navigator.clipboard.writeText('base: &defaults\n  value: preserved\ncopy: *'));
  await page.keyboard.press('ControlOrMeta+V');
  await page.keyboard.press('ControlOrMeta+End');
  await page.keyboard.press('Control+Space');
  await page.locator('.suggest-widget.visible').waitFor();
  await page.locator('.suggest-widget.visible .monaco-list-row').filter({ hasText: 'defaults' }).waitFor();
  await page.keyboard.press('Escape');
  await page.keyboard.press('ControlOrMeta+f');
  await page.locator('.find-widget.visible').waitFor();
  await page.keyboard.press('Escape');
  assert(workers.some(url => url.includes('yaml.worker')), 'YAML validation worker did not start');
  assert.equal(workers.filter(url => url.includes('yaml.worker')).length, 1, 'warm opens recreated the YAML worker');
  assert.deepEqual(external, [], 'editor requested an external/CDN resource');
  assert.deepEqual(errors, []);
  const assets = await page.evaluate(() => performance.getEntriesByType('resource').filter(entry => entry.name.includes('/assets/')).map(entry => ({ name: entry.name.split('/').pop(), bytes: entry.decodedBodySize, duration: Math.round(entry.duration) })));
  console.log(JSON.stringify({ openMs: timings, workers: workers.map(url => url.split('/').pop()), editorAssets: assets.filter(asset => /YamlEditor|editor|yaml|monaco/i.test(asset.name)), assetRequests: requests.filter(url => url.includes('/assets/')).length }, null, 2));
  console.log('PASS: local production editor, repeated opens, six locale drafts, YAML validation/completion and find');
} finally {
  await browser.close();
  if (server) await new Promise(resolve => server.httpServer.close(resolve));
}
