/** Isolated notification API/UI acceptance. All deliveries terminate at a local fixture. */
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, open, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createServer } from 'node:http';
import { chromium } from 'playwright';

const work = await mkdtemp(join(tmpdir(), 'notification-center-'));
const evidence = resolve('data/maintenance/notification-center'); await mkdir(evidence, { recursive: true });
const translations = JSON.parse(await readFile('src/shared/i18n/locales/notifications.json', 'utf8'));
const text = name => translations[name][0];
const report = { work, checks: [], screenshots: [] };
const pass = name => { report.checks.push(name); console.log('PASS', name); };
async function wait(fn, seconds = 40) {
  const deadline = Date.now() + seconds * 1000;
  while (Date.now() < deadline) { if (await fn()) return; await new Promise(r => setTimeout(r, 200)); }
  throw Error('Acceptance timed out');
}
const deliveries = [];
let responseStatus = 200;
const receiver = createServer(async (request, response) => {
  let body = ''; for await (const part of request) body += part;
  deliveries.push({ body: JSON.parse(body), authorization: request.headers.authorization });
  response.writeHead(responseStatus, { 'Content-Type': 'application/json' }); response.end('{}');
});
await new Promise(r => receiver.listen(0, '127.0.0.1', r));
const webhook = `http://127.0.0.1:${receiver.address().port}/notify`;
const portProbe = createServer(); await new Promise(r => portProbe.listen(0, '127.0.0.1', r));
const bind = `127.0.0.1:${portProbe.address().port}`, origin = `http://${bind}`; await new Promise(r => portProbe.close(r));
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: bind };
for (const key of ['FERRY_DATABASE_URL', 'FERRY_NATS_URL', 'FERRY_CLUSTER_NAMESPACE']) delete env[key];
const log = await open(join(work, 'server.log'), 'a');
let server, browser;
try {
  server = spawn(resolve(process.env.FERRY_BINARY || `target/debug/ferry-server${process.platform === 'win32' ? '.exe' : ''}`), [], { env, windowsHide: true, stdio: ['ignore', log.fd, log.fd] });
  await wait(async () => { assert.equal(server.exitCode, null); try { return (await fetch(origin + '/api/status')).ok; } catch { return false; } });
  browser = await chromium.launch({ channel: 'chrome', headless: true });
  const owner = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  const anonymous = await browser.newContext();
  const api = context => async (path, data, method = data ? 'POST' : 'GET', status) => {
    const response = await context.request.fetch(origin + '/api' + path, { method, headers: { 'X-Ferry-Request': '1', 'Accept-Language': 'en-US' }, ...(data ? { data } : {}) });
    assert(status ? response.status() === status : response.ok(), `${method} ${path}: ${response.status()} ${await response.text()}`);
    return response.status() === 204 ? null : response.json();
  };
  const admin = api(owner), publicApi = api(anonymous);
  await admin('/setup', { name: 'Notification owner', email: 'owner@example.invalid', password: 'temporary-notification-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace = await admin('/workspaces', { name: 'Notification acceptance', slug: 'notification-acceptance' });
  const second = await admin('/workspaces', { name: 'Other space', slug: 'notification-other' });
  const root = `/workspaces/${workspace.id}`, channels = `${root}/notification-channels`;
  const channel = await admin(channels, { name: 'Local delivery fixture', provider: 'Webhook', webhook, bearer_token: 'fixture-private', language: 'en-US' });
  const view = await admin(`${channels}/${channel.id}`);
  assert.equal(view.has_webhook, true); assert.equal(view.has_secret, true);
  assert(!JSON.stringify(view).includes('fixture-private')); assert(!JSON.stringify(view).includes(webhook));
  await publicApi(channels, undefined, 'GET', 401);
  await admin(`/workspaces/${second.id}/notification-channels/${channel.id}`, undefined, 'GET', 404);
  pass('Channel secret projection and workspace/anonymous isolation');
  const body = { client_id: 'fixture-idempotency', subject: 'Build report', body: 'Line one\nLine two' };
  const [first, replay] = await Promise.all([admin(`${channels}/${channel.id}/deliveries`, body), admin(`${channels}/${channel.id}/deliveries`, body)]);
  assert.equal(first.id, replay.id); assert.equal(first.state === 'succeeded' || replay.state === 'succeeded', true);
  assert.equal(deliveries.length, 1); assert.equal(deliveries[0].authorization, 'Bearer fixture-private');
  await admin(`${channels}/${channel.id}/deliveries`, { ...body, body: 'conflict' }, 'POST', 409);
  pass('Real HTTP delivery, credential use and concurrent idempotency');
  responseStatus = 400;
  const failed = await admin(`${channels}/${channel.id}/deliveries`, { ...body, client_id: 'fixture-failure' });
  assert.equal(failed.state, 'failed');
  await admin(`${channels}/${channel.id}/deliveries`, { ...body, client_id: 'fixture-failure' });
  assert.equal(deliveries.length, 2); responseStatus = 200;
  pass('Explicit failure stays recorded without replay');
  const template = await admin(`${root}/notification-templates`, { name: 'Release report', language: 'en-US', subject: 'Release {{version}}', body: '<h1>{{version}}</h1><p>{{content}}</p>', format: 'html' });
  const templated = await admin(`${channels}/${channel.id}/deliveries`, { client_id: 'template', template_id: template.id, variables: { version: '1.0', content: '<unsafe>' } });
  assert.equal(templated.body, '<h1>1.0</h1><p>&lt;unsafe&gt;</p>');
  const page = await owner.newPage(), errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const route = (resource, suffix = '') => `${origin}/en-US${root}/${resource}${suffix}`;
  await page.goto(route('notification-channels', '/new'));
  await page.getByLabel(text('name'), { exact: true }).fill('Created in UI');
  await page.getByRole('combobox', { name: text('provider'), exact: true }).click();
  await page.getByRole('option', { name: 'Webhook', exact: true }).click();
  await page.getByLabel(text('webhook'), { exact: true }).fill(webhook);
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await wait(async () => (await admin(channels)).items.some(row => row.name === 'Created in UI'));
  pass('Shared CRUD channel creation through real UI');
  const paths = [
    ['notification-channels', `/${channel.id}`], ['notification-channels', `/${channel.id}/edit`],
    ['notification-templates', `/${template.id}`], ['notification-deliveries', `/${templated.id}`],
  ];
  for (const width of [1440, 900, 390, 320]) {
    await page.setViewportSize({ width, height: 960 });
    for (const [resource, suffix] of paths) {
      await page.goto(route(resource, suffix));
      await page.getByRole('main').waitFor();
      await wait(async () => !(await page.getByText('Loading…', { exact: true }).count()));
      await page.getByText(resource === 'notification-templates' ? 'Release report' : resource === 'notification-deliveries' ? 'Release 1.0' : 'Local delivery fixture', { exact: true }).first().waitFor();
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${resource} width ${width}`);
      const screenshot = `${resource}-${width}${suffix.endsWith('/edit') ? '-edit' : ''}.png`;
      await page.screenshot({ path: join(evidence, screenshot), fullPage: true }); report.screenshots.push(screenshot);
    }
  }
  assert.equal(errors.length, 0, errors.join('\n'));
  pass('Notification view/edit/template/delivery routes at four widths without page overflow or JS errors');
  for (const locale of ['zh-CN', 'ms-MY', 'en-US']) {
    for (const width of [1440, 390]) {
      await page.emulateMedia({ colorScheme: 'dark' });
      await page.setViewportSize({ width, height: 960 });
      await page.goto(`${origin}/${locale}${root}/notification-channels/${channel.id}/edit`);
      await page.locator('input[name="name"]').waitFor();
      await wait(async () => (await page.locator('input[name="name"]').inputValue()) === 'Local delivery fixture');
      assert(await page.evaluate(() => document.documentElement.classList.contains('dark')), 'dark scheme applied');
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${locale} dark ${width}`);
      const screenshot = `notification-${locale}-dark-${width}.png`;
      await page.screenshot({ path: join(evidence, screenshot), fullPage: true }); report.screenshots.push(screenshot);
    }
  }
  assert.equal(errors.length, 0, errors.join('\n'));
  pass('Chinese, English and Malay dark-mode pages at desktop and phone widths');
  const editable = await admin(`${channels}/${channel.id}`);
  await admin(`${channels}/${channel.id}`, { name: editable.name, provider: editable.provider, enabled: false, language: editable.language, version: editable.version }, 'PATCH');
  await admin(`${channels}/${channel.id}/deliveries`, { ...body, client_id: 'disabled' }, 'POST', 409);
  assert.equal(deliveries.length, 3);
  pass('Disabled channel blocks further side effects');
  report.result = 'PASS';
} catch (error) {
  report.result = 'FAIL'; report.error = String(error.stack || error); throw error;
} finally {
  await writeFile(join(evidence, 'acceptance.json'), JSON.stringify(report, null, 2));
  await browser?.close();
  if (server?.exitCode === null) { const ended = once(server, 'exit'); server.kill(); await ended; }
  await log.close(); await new Promise(r => receiver.close(r));
}
