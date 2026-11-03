/** Shipped UI: run logs and jobs scroll independently; saved configuration is linkable. */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = process.env.RUN_DETAIL_UI_URL ? null : await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.RUN_DETAIL_UI_URL || server.resolvedUrls.local[0].replace(/\/$/, '');
const chrome = process.env.PLAYWRIGHT_CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const browser = await chromium.launch({ headless: true, ...(existsSync(chrome) ? { executablePath: chrome } : {}) });
try {
  const page = await browser.newPage({ locale: 'zh-CN' });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const workflow = 'name: Original workflow\non: workflow_dispatch\njobs:\n  build:\n    runs-on: self-hosted\n    steps:\n      - run: echo original\n';
  const run = { id: '51', number: 7, state: 'succeeded', pipeline_id: '42', pipeline_name: 'Source pipeline', host_id: '0', triggered_by: '1', triggered_by_name: 'Operator', started_at: 1, finished_at: 5000, commit: 'abc',
    definition_snapshot: { workflow, executor_bindings: [], checkout_bindings: [] },
    jobs: Array.from({ length: 40 }, (_, i) => ({ key: `job-${i}`, name: `Job ${i}`, state: 'succeeded', host_id: '0', step_indices: [0, 1], started_at: 1, finished_at: 5000 })),
    steps: [0, 1].map(index => ({ index, name: `Step ${index}`, state: 'succeeded', exit_code: 0, started_at: 1, finished_at: 5000 })) };
  const chunks = [0, 1].map(index => ({ seq: index + 1, step_index: index, stream: 'stdout', chunk: Array.from({ length: 400 }, (_, i) => `line ${i}`).join('\n') }));
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname;
    let json = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/setup') json = { required: false, has_users: true };
    if (path === '/api/me') json = { user: { id: '1', name: 'Operator' }, workspaces: [{ id: '2', name: 'Operations', slug: 'ops', permissions: ['pipelines.read'], role_name: 'Viewer' }] };
    if (path.endsWith('/preferences')) json = { default_page_size: 20, dictionaries: {} };
    if (path === '/api/workspaces/2/pipeline-runs/51') json = run;
    if (path.endsWith('/51/logs')) json = { items: chunks, next_seq: 2 };
    await route.fulfill({ json });
  });
  const url = origin + '/zh-CN/workspaces/2/pipeline-runs/51';
  for (const width of [1440, 768, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(url);
    const logs = page.getByRole('region', { name: '执行日志', exact: true });
    await logs.waitFor();
    const jobs = page.getByRole('complementary', { name: '任务列表', exact: true });
    const content = page.locator('section[aria-label="任务步骤和日志"]');
    const source = page.getByRole('link', { name: 'Source pipeline', exact: true });
    assert.equal(new URL(await source.getAttribute('href'), origin).pathname, '/zh-CN/workspaces/2/pipelines/42');
    const geometry = () => page.evaluate(() => {
      const jobs = document.querySelector('aside[aria-label="任务列表"]');
      const summary = document.querySelector('section[aria-label="执行概况"]');
      const bounds = el => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, height: r.height }; };
      return { jobs: bounds(jobs), jobsScroll: jobs.scrollTop, summary: summary ? bounds(summary) : null, documentScroll: document.scrollingElement.scrollTop };
    });
    assert(await content.evaluate(el => el.clientHeight > 80), `${width}: visible log workspace`);
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `${width}: no horizontal page overflow`);
    await logs.evaluate(el => { el.scrollTop = 0; });
    const before = await geometry();
    await logs.hover();
    await page.mouse.wheel(0, 300);
    await page.waitForFunction(() => document.querySelector('[aria-label="执行日志"]').scrollTop > 0);
    assert.deepEqual(await geometry(), before, `${width}: scrolling logs leaves job list and page fixed`);
    // A second wheel at the bottom must not chain into the right pane or page.
    await logs.evaluate(el => { el.scrollTop = el.scrollHeight; });
    const contentScroll = await content.evaluate(el => el.scrollTop);
    await page.mouse.wheel(0, 1000);
    await page.waitForTimeout(150);
    assert.equal(await content.evaluate(el => el.scrollTop), contentScroll, `${width}: log overscroll is contained`);
    assert.deepEqual(await geometry(), before);
    const logScroll = await logs.evaluate(el => el.scrollTop);
    await jobs.hover();
    await page.mouse.wheel(0, 300);
    await page.waitForFunction(() => document.querySelector('aside[aria-label="任务列表"]').scrollTop > 0);
    assert.equal(await logs.evaluate(el => el.scrollTop), logScroll, `${width}: job scrolling leaves logs fixed`);
    const jobsBounds = await jobs.boundingBox();
    await content.evaluate(el => { el.scrollTop = 200; });
    assert.deepEqual(await jobs.boundingBox(), jobsBounds, `${width}: steps pane scroll leaves jobs fixed`);
    await page.getByRole('tab', { name: '配置快照', exact: true }).click();
    assert.equal(new URL(page.url()).searchParams.get('tab'), 'snapshot');
    assert.equal(await page.locator('pre[aria-label="工作流快照"]').textContent(), workflow);
    await page.reload();
    await page.locator('pre[aria-label="工作流快照"]').waitFor();
    assert.equal(await page.locator('pre[aria-label="工作流快照"]').textContent(), workflow, 'snapshot survives reload');
    await page.getByRole('tab', { name: '任务步骤和日志', exact: true }).click();
    await logs.waitFor();
    console.log(`PASS ${width}px: independent log/job/steps scrolling, contained overscroll, pipeline link, snapshot deep link and reload`);
  }
  run.definition_snapshot = null;
  await page.goto(url + '?tab=snapshot');
  await page.getByText('此历史运行没有可用的配置快照。', { exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log('PASS legacy snapshot absence and no browser errors');
} finally {
  await browser.close();
  if (server) await new Promise(resolve => server.httpServer.close(resolve));
}
