/** Production Chromium acceptance for execution editor/detail locale state. */
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { preview } from 'vite';
import { chromium } from 'playwright';
const catalog = JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/execution-domain.json', import.meta.url), 'utf8'));
const server = process.env.EXECUTION_UI_URL ? null : await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.EXECUTION_UI_URL || server.resolvedUrls.local[0].replace(/\/$/, '');
const chrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const browser = await chromium.launch({ headless: true, ...(existsSync(chrome) ? { executablePath: chrome } : {}) });
const page = await browser.newPage({ viewport: { width: 1366, height: 900 } });
const errors = [], requests = [];
page.on('pageerror', error => errors.push(error.message));
page.setDefaultTimeout(15000);
const definition = 'name: User workflow\non: workflow_dispatch\njobs:\n  build:\n    name: User job\n    runs-on: self-hosted\n    steps:\n      - name: User step\n        run: echo user\n';
const run = { id: '51', number: 7, state: 'succeeded', pipeline_id: '42', pipeline_name: 'User pipeline', host_id: '7', host_name: 'User host', triggered_by: '1', triggered_by_name: 'Operator', started_at: 1, finished_at: 5000, commit: 'abc', jobs: [{ key: 'build', name: 'User job', state: 'succeeded', step_indices: [0], started_at: 1, finished_at: 5000 }], steps: [{ index: 0, name: 'User step', state: 'succeeded', exit_code: 0, started_at: 1, finished_at: 5000 }] };
const rawLog = 'User log / 用户日志 / 日本語ログ';
await page.route('**/api/**', async route => {
  const request = route.request(), path = new URL(request.url()).pathname.slice(4);
  requests.push({ path, method: request.method() });
  let body = { items: [], total: 0, page: 1, page_size: 200 };
  if (path === '/me') body = { user: { id: '1', name: 'Operator', email: 'operator@example.invalid', is_superadmin: true }, workspaces: [{ id: '2', name: 'Operations', slug: 'ops', permissions: ['pipelines.read', 'pipelines.update', 'pipelines.run', 'hosts.read', 'host-groups.read', 'deployments.read', 'deployments.apply'], role_name: 'Owner' }] };
  if (path === '/setup') body = { required: false, has_users: true, registration_enabled: true };
  if (path.endsWith('/preferences')) body = { default_page_size: 20, dictionaries: {} };
  if (path === '/workspaces/2/pipelines/42') body = { id: '42', name: 'User pipeline', description: '', version: 1, definition_format: 'github-actions', definition, executor_bindings: [{ job_id: 'build', target_kind: 'host', target_ref: '7', image: 'host' }], checkout_bindings: [], graph_layout: {} };
  if (path === '/workspaces/2/nodes') body = { items: [{ id: '7', name: 'User host' }], total: 1, page: 1, page_size: 200 };
  if (path === '/workspaces/2/pipeline-runs/51') body = run;
  if (path === '/workspaces/2/pipeline-runs/51/logs') body = { items: [{ seq: 1, step_index: 0, stream: 'stdout', chunk: rawLog }], next_seq: 1 };
  if (path === '/workspaces/2/deployments/61') body = { id: '61', name: 'User application', state: 'needs-review', target_ref: '7', version: 1, latest_revision: { id: '71', number: 3, action: 'apply', state: 'needs-review', output: rawLog, artifact: 'services: {}' } };
  await route.fulfill({ json: body });
});
try {
  await page.goto(origin + '/en-US/workspaces/2/pipelines/42/edit');
  await page.getByRole('button', { name: 'User job', exact: true }).click();
  const name = page.getByRole('textbox', { name: catalog['en-US']['editor.jobName'], exact: true });
  await name.fill('Unsaved job name');
  for (const locale of Object.keys(catalog)) {
    await page.locator('header select').first().selectOption(locale);
    await page.getByRole('heading', { name: catalog[locale]['editor.jobSettings'], exact: true }).waitFor();
    assert.equal(await page.getByRole('textbox', { name: catalog[locale]['editor.jobName'], exact: true }).inputValue(), 'Unsaved job name');
    assert.equal(await page.getByRole('combobox', { name: catalog[locale]['editor.executor'], exact: true }).inputValue(), '7');
    assert.equal(await page.getByRole('combobox', { name: catalog[locale]['editor.executor'], exact: true }).locator('option:checked').textContent(), 'User host');
  }
  for (const width of [1366, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const selector = page.locator('header select').first();
    const box = await selector.boundingBox();
    assert(box && box.x >= 0 && box.x + box.width <= width + 1, `${width}px pipeline locale selector outside viewport`);
    const overlaps = await selector.evaluate(element => {
      const own = element.getBoundingClientRect();
      return [...element.closest('header').querySelectorAll('button, a')].some(other => {
        const rect = other.getBoundingClientRect();
        return rect.width && rect.height && Math.min(own.right, rect.right) > Math.max(own.left, rect.left) && Math.min(own.bottom, rect.bottom) > Math.max(own.top, rect.top);
      });
    });
    assert.equal(overlaps, false, `${width}px pipeline locale selector overlaps an action`);
  }
  await page.setViewportSize({ width: 1366, height: 900 });
  await page.goto(origin + '/en-US/workspaces/2/pipeline-runs/51');
  await page.getByText(rawLog, { exact: false }).waitFor();
  const runLoads = requests.filter(request => request.path.includes('/pipeline-runs/51')).length;
  for (const locale of Object.keys(catalog)) {
    await page.locator('header select').first().selectOption(locale);
    await page.getByText(catalog[locale]['run.logsLoaded'], { exact: true }).waitFor();
    assert((await page.locator('body').innerText()).includes(rawLog));
  }
  assert.equal(requests.filter(request => request.path.includes('/pipeline-runs/51')).length, runLoads);
  await page.goto(origin + '/en-US/workspaces/2/deployments/61');
  await page.locator('#review-note').fill('Unsaved review note');
  const deploymentLoads = requests.filter(request => request.path === '/workspaces/2/deployments/61').length;
  for (const locale of Object.keys(catalog)) {
    await page.locator('header select').first().selectOption(locale);
    await page.getByText(catalog[locale]['deployment.reviewHelp'], { exact: true }).waitFor();
    assert.equal(await page.locator('#review-note').inputValue(), 'Unsaved review note');
    assert((await page.locator('body').innerText()).includes(rawLog));
  }
  assert.equal(requests.filter(request => request.path === '/workspaces/2/deployments/61').length, deploymentLoads);
  assert.deepEqual(errors, []);
  console.log('PASS: real workflow graph selection and unsaved YAML name, executor identity, loaded run logs, deployment review note and raw output retained in all six languages without refetching details.');
} finally {
  await browser.close();
  if (server) await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
