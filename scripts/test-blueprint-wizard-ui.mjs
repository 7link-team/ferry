/**
 * Real Chromium: blueprint deployment wizard from the market to the detail topology.
 * Three hosts carry a tag (a fourth does not) → the market's nats-cluster "Deploy" → every
 * section of the continuous configuration form (basics, target with driver/roles/placement,
 * parameters; the former numbered steps were merged by design) → YAML round trip (an invalid YAML locks the form) → placement preview with one
 * host changed by hand (addresses stay read-only) → create → detail topology. Repeated at
 * 1440/900/390/320 with screenshots, no page-level horizontal scroll, and the sticky bottom
 * action visible and clickable. Creation only: nothing is deployed, so no Docker is needed.
 *
 * Run after `npm run build` and `cargo build -p ferry-server`.
 */
import assert from 'node:assert/strict';
import { createServer as createHttpServer } from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, dirname } from 'node:path';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';
import { parse } from 'yaml';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const output = resolve(process.env.FERRY_WIZARD_UI_OUTPUT || join(repo, 'data', 'maintenance', 'orchestration-p2c'));
await mkdir(output, { recursive: true });
const work = await mkdtemp(join(tmpdir(), 'blueprint-wizard-ui-'));
const listener = createServer();
await new Promise(r => listener.listen(0, '127.0.0.1', r));
const port = listener.address().port;
await new Promise(r => listener.close(r));
const origin = `http://127.0.0.1:${port}`;
// The official index exactly as bundled, served like a GitHub contents API.
const bundle = JSON.parse(await readFile(join(repo, 'marketplace', 'index.json'), 'utf8'));
const fixture = createHttpServer((request, response) => {
  response.setHeader('Content-Type', 'application/json');
  response.end(JSON.stringify({ encoding: 'base64', content: Buffer.from(JSON.stringify(bundle)).toString('base64') }));
});
await new Promise(r => fixture.listen(0, '127.0.0.1', r));
const fixtureOrigin = `http://127.0.0.1:${fixture.address().port}`;
const log = await open(join(work, 'server.log'), 'w');
const env = { ...process.env, FERRY_DATA_DIR: work, FERRY_BIND: `127.0.0.1:${port}`, FERRY_TEST_MARKETPLACE_HTTP: '1' };
delete env.FERRY_DATABASE_URL;
const server = spawn(resolve(repo, process.env.FERRY_BINARY || 'target/debug/ferry-server'), [], { env, stdio: ['ignore', log.fd, log.fd], windowsHide: true });
let browser, passed = false;
const checks = [];
const check = (condition, label) => { assert(condition, label); checks.push(label); console.log('  ok', label); };
async function eventually(fn, label, tries = 300) {
  for (let i = 0; i < tries; i++) { const result = await fn(); if (result) return result; await new Promise(r => setTimeout(r, 100)); }
  throw new Error('timed out: ' + label);
}
try {
  await eventually(async () => { assert.equal(server.exitCode, null, 'server exited'); try { return (await fetch(`${origin}/api/status`)).ok; } catch { return false; } }, 'server startup', 200);
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const headers = { 'X-Ferry-Request': '1', 'Accept-Language': 'zh-CN' };
  const context = await browser.newContext({ locale: 'zh-CN', extraHTTPHeaders: { 'Accept-Language': 'zh-CN' }, viewport: { width: 1440, height: 900 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const request = async (method, path, data) => {
    const response = await context.request.fetch(`${origin}/api${path}`, { method, headers, ...(data === undefined ? {} : { data }) });
    assert(response.ok(), `${method} ${path}: ${response.status()} ${await response.text()}`);
    return response.status() === 204 ? undefined : response.json();
  };
  const get = path => request('GET', path);
  const post = (path, data) => request('POST', path, data);
  await post('/setup', { email: 'admin@example.invalid', name: 'Admin', password: 'temporary-admin-password', token: (await readFile(join(work, 'setup-token'), 'utf8')).trim() });
  const workspace = (await post('/workspaces', { name: 'Orchestration', slug: 'orchestration' })).id;
  const root = `/workspaces/${workspace}`;

  // The official templates as a platform source, synchronized from the local fixture.
  const source = await post('/platform/marketplace-sources', { name: 'Official fixture', trust: 'official', enabled: true, sync_interval_hours: 6, provider: 'github', api_base: fixtureOrigin, project: 'ferry/ferry', branch: 'main', index_path: 'marketplace/index.json' });
  await post(`/platform/marketplace-sources/${source.id}/synchronizations`, {});
  await eventually(async () => (await get(`/platform/marketplace-sources/${source.id}`)).entry_count > 0, 'platform source sync');

  // Three tagged hosts and one untagged. They are fixtures for placement only: online Linux
  // hosts with addresses, recorded directly because creation never contacts them.
  const tag = await post(`${root}/tags`, { name: 'nats-ready' });
  const hosts = [];
  for (const [index, tagged] of [[0, true], [1, true], [2, true], [3, false]]) {
    const created = await post(`${root}/nodes`, { name: `bp-host-${index}`, connection: 'runner', tag_ids: tagged ? [tag.id] : [] });
    hosts.push({ id: created.id, name: `bp-host-${index}`, address: `192.0.2.${10 + index}`, tagged });
  }
  const db = new DatabaseSync(join(work, 'ferry.sqlite'));
  for (const host of hosts) db.prepare("UPDATE hosts SET state='online', os='linux', address=? WHERE id=?").run(host.address, BigInt(host.id));
  db.close();
  const tagged = hosts.filter(h => h.tagged);
  const listedHosts = (await get(`${root}/nodes?page_size=50`)).items;
  check(tagged.every(h => listedHosts.find(row => row.id === h.id)?.tags?.some(t => t.name === 'nats-ready')) && listedHosts.find(row => row.id === hosts[3].id)?.tags?.length === 0, 'three hosts carry the tag, the fourth does not');

  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.stack ?? e.message));
  page.on('dialog', d => d.accept());
  // The blueprint wizard is one continuous form (ScrollFormSections): basics, deployment target
  // (driver, roles and hosts, placement preview), the read-only definition and template parameters.
  const basics = page.getByRole('region', { name: '基本信息', exact: true });
  const target = page.getByRole('region', { name: '部署目标', exact: true });
  const params = page.getByRole('region', { name: '模板参数', exact: true });
  const yamlEditor = params.locator('.monaco-editor .view-lines');
  const readYaml = async () => { await yamlEditor.click(); await page.keyboard.press('ControlOrMeta+A'); await page.keyboard.press('ControlOrMeta+C'); return (await page.evaluate(() => navigator.clipboard.readText())).replace(/\r\n/g, '\n'); };
  const setYaml = async value => { await yamlEditor.click(); await page.keyboard.press('ControlOrMeta+A'); await page.evaluate(text => navigator.clipboard.writeText(text), value); await page.keyboard.press('ControlOrMeta+V'); };
  const actions = page.locator('[data-form-actions]');
  const createButton = actions.getByRole('button', { name: '创建部署', exact: true });
  // The single bottom action is enabled once every section is valid and the placement preview is current.
  const ready = label => eventually(() => createButton.isEnabled(), label);
  const fits = async label => check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${label}: no page-level horizontal scroll`);
  /** The sticky bottom action is inside the viewport, on top at its center, and enabled. */
  const reachable = async (button, label) => {
    await button.waitFor();
    const info = await button.evaluate(element => {
      const box = element.getBoundingClientRect();
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return { ok: box.top >= 0 && box.bottom <= innerHeight && box.left >= 0 && box.right <= innerWidth && !!hit && element.contains(hit), top: box.top, bottom: box.bottom, left: box.left, right: box.right, innerHeight, innerWidth, hit: hit?.outerHTML?.slice(0, 300) };
    });
    const onTop = info.ok;
    if (!onTop) { console.log('unreachable action', JSON.stringify(info)); await page.screenshot({ path: join(output, `unreachable-${label.replace(/\W+/g, '-')}.png`) }); }
    check(onTop && await button.isEnabled(), `${label}: bottom action visible, uncovered and enabled`);
  };
  /** The page scrolls inside its main content area; a long page also gets its end captured. */
  const scrollMain = to => page.evaluate(to => {
    const area = [...document.querySelectorAll('main *')].filter(element => /auto|scroll/.test(getComputedStyle(element).overflowY) && element.scrollHeight > element.clientHeight + 1)
      .sort((a, b) => b.clientHeight - a.clientHeight)[0];
    if (!area) return false;
    area.scrollTop = to === 'end' ? area.scrollHeight : 0;
    return true;
  }, to);
  const shot = async name => {
    await page.screenshot({ path: join(output, `${name}.png`), fullPage: true });
    if (await scrollMain('end')) { await page.screenshot({ path: join(output, `${name}-end.png`), fullPage: true }); await scrollMain('start'); }
  };

  for (const width of [1440, 900, 390, 320]) {
    console.log(`== ${width}px`);
    await page.setViewportSize({ width, height: width < 768 ? 740 : 900 });
    const clientPort = 20000 + width;

    // Market: the cluster template offers Deploy, which imports it and opens the wizard.
    await page.goto(`${origin}/zh-CN/workspaces/${workspace}/marketplace?q=NATS`);
    const entry = page.getByRole('row').filter({ hasText: 'NATS JetStream 集群' }).first();
    await entry.waitFor();
    await fits(`market ${width}`);
    await shot(`market-${width}`);
    const deploy = entry.getByRole('button', { name: '部署', exact: true });
    if (await deploy.isVisible()) await deploy.click();
    else { await entry.getByRole('button').last().click(); await page.getByRole('menuitem', { name: '部署', exact: true }).click(); }
    await page.waitForURL(url => url.pathname.endsWith('/deployments/new') && url.searchParams.get('wizard') === 'blueprint');
    const templateId = new URL(page.url()).searchParams.get('template');
    const template = await get(`${root}/app-templates/${templateId}`);
    check(JSON.parse(template.content).metadata?.name === 'nats-cluster', `${width}: market Deploy imported nats-cluster and opened the wizard for it`);

    // ① Template and version: the version is in the basics section, driver and roles in the target section.
    const templateStep = target.locator('section[aria-label="模板与版本"]');
    await templateStep.waitFor();
    check(await basics.getByText(`v${template.version}`, { exact: true }).isVisible() && await basics.getByText(template.name, { exact: true }).isVisible() && await templateStep.getByText('nats', { exact: true }).isVisible(), `${width}: template section shows the template, its version and the nats role`);
    // Docker: one instance per host, on the hosts the scheduler picks.
    const driver = templateStep.locator('select').first();
    await driver.selectOption('docker');
    check(await driver.inputValue() === 'docker', `${width}: the docker driver is chosen in the form`);
    await fits(`wizard template ${width}`);
    await shot(`wizard-1-template-${width}`);
    await ready('template section ready');
    await reachable(createButton, `${width} template section`);

    // ② Values, then the YAML round trip from this section.
    check(await params.locator('section[aria-label="参数"]').isVisible(), `${width}: values section is present`);
    const clientInput = params.locator('[name="client_port"]');
    await clientInput.waitFor();
    check(await clientInput.inputValue() === '14222', `${width}: values default from the template parameters`);
    await fits(`wizard values ${width}`);
    await shot(`wizard-2-values-${width}`);
    await reachable(createButton, `${width} values section`);

    await params.getByRole('button', { name: 'YAML', exact: true }).click();
    await yamlEditor.waitFor();
    const fromForm = await readYaml();
    const spec = parse(fromForm);
    check(spec.apiVersion === 'ferry/deploy-v1' && String(spec.template.id) === String(template.id) && spec.template.version === template.version && spec.roles.nats.instances === 3 && spec.driver === 'docker',
      `${width}: form → YAML carries apiVersion, template, driver and role instances`);
    const edited = `# chosen by the acceptance run\n${fromForm.replace(/client_port: \d+/, `client_port: ${clientPort}`)}`;
    assert(edited.includes(`client_port: ${clientPort}`), 'client_port line present in the spec');
    await setYaml(edited);
    await fits(`wizard yaml ${width}`);
    await shot(`wizard-yaml-${width}`);
    await params.getByRole('button', { name: '表单', exact: true }).click();
    await clientInput.waitFor();
    check(await clientInput.inputValue() === String(clientPort), `${width}: YAML → form updates the parameter`);
    await params.getByRole('button', { name: 'YAML', exact: true }).click();
    await yamlEditor.waitFor();
    check((await readYaml()).startsWith('# chosen by the acceptance run'), `${width}: the YAML comment survives the round trip`);

    // An unknown field makes the YAML invalid: the error shows and the form is locked.
    const valid = await readYaml();
    await setYaml(`${valid}\nunknownSpecKey: true\n`);
    const yamlAlert = page.getByRole('alert').filter({ hasText: '未知字段' });
    await yamlAlert.waitFor();
    await params.getByRole('button', { name: '表单', exact: true }).click();
    const locked = page.getByRole('alert').filter({ hasText: 'YAML 规格有错误' });
    await locked.waitFor();
    check(await clientInput.isDisabled() && await createButton.isDisabled() && await clientInput.inputValue() === String(clientPort),
      `${width}: invalid YAML locks the form, keeps the last valid values and blocks creation`);
    await fits(`wizard yaml invalid ${width}`);
    await shot(`wizard-yaml-invalid-${width}`);
    await locked.getByRole('button', { name: '放弃 YAML 修改', exact: true }).click();
    await locked.waitFor({ state: 'detached' });
    await ready('form unlocked');
    check(await clientInput.isEnabled() && await createButton.isEnabled(), `${width}: discarding the YAML edit unlocks the form`);

    // ③ Roles and hosts: the tag selector through the form.
    check(await target.locator('section[aria-label="角色与主机"]').isVisible(), `${width}: roles section is present`);
    const failureMode = page.getByRole('combobox', { name: '故障处理', exact: true });
    check(await failureMode.inputValue() === 'auto', `${width}: failure handling defaults to automatic`);
    check(await page.getByText('主机失联阈值：90 秒。可在 YAML 中修改 hostDownAfter。', { exact: true }).isVisible(), `${width}: host silence threshold is read-only guidance`);
    await failureMode.selectOption('alert');
    if (width < 1200) check((await failureMode.boundingBox()).height >= 44, `${width}: failure control has a 44px touch target`);
    const tagSelect = page.locator('select[name="nats-tags"]');
    await tagSelect.selectOption(['nats-ready']);
    check(await page.locator('output[aria-label="实例数"]').first().innerText() === '3', `${width}: nats keeps three instances`);
    check(await page.getByRole('button', { name: '减少一个 nats 实例' }).isDisabled(), `${width}: instances cannot go below the blueprint minimum`);
    await fits(`wizard roles ${width}`);
    await shot(`wizard-3-roles-${width}`);
    await ready('roles section ready');
    await reachable(createButton, `${width} roles section`);

    // ④ Placement preview: exactly the tagged hosts, then one host changed by hand.
    check(await target.locator('section[aria-label="放置预览"]').isVisible(), `${width}: placement section is present`);
    const placement = page.getByRole('region', { name: '放置结果' });
    await placement.waitFor();
    const hostOf = instance => page.getByRole('combobox', { name: `${instance} 的主机`, exact: true });
    // A table row on Desktop/Pad, a card on Phone.
    const rowOf = instance => placement.locator('tr, li').filter({ has: hostOf(instance) });
    check(width < 768 ? await placement.locator('table').count() === 0 && await placement.locator('[data-placement-instance]').count() === 3
      : await placement.locator('table').count() === 1, `${width}: placement is ${width < 768 ? 'one card per instance' : 'a table'}`);
    await hostOf('nats-2').waitFor();
    const before = [];
    for (const instance of ['nats-0', 'nats-1', 'nats-2']) before.push(await hostOf(instance).inputValue());
    check(new Set(before).size === 3 && before.every(id => tagged.some(h => h.id === id)) && !before.includes(hosts[3].id),
      `${width}: automatic placement uses exactly the three tagged hosts, one instance each`);
    for (const [index, instance] of ['nats-0', 'nats-1', 'nats-2'].entries())
      check((await rowOf(instance).innerText()).includes(hosts.find(h => h.id === before[index]).address), `${width}: ${instance} address is its host's address`);
    const moved = before[1];
    const settled = page.waitForResponse(response => response.url().endsWith('/deployment-placements') && response.request().method() === 'POST');
    await hostOf('nats-0').selectOption(moved);
    check((await settled).ok(), `${width}: changing a host recalculates the placement`);
    await eventually(async () => await hostOf('nats-0').inputValue() === moved && await hostOf('nats-1').inputValue() === before[0], 'manual swap applied');
    check(await rowOf('nats-0').getByText('人工', { exact: true }).isVisible(), `${width}: the role is marked as manually placed`);
    const movedAddress = hosts.find(h => h.id === moved).address;
    check((await rowOf('nats-0').innerText()).includes(movedAddress), `${width}: nats-0 address follows the chosen host`);
    check(await rowOf('nats-0').locator('input, textarea, [contenteditable="true"]').count() === 0, `${width}: the address is read-only`);
    await fits(`wizard placement ${width}`);
    await shot(`wizard-4-placement-${width}`);
    await ready('placement section ready');
    await reachable(createButton, `${width} placement section`);

    // ⑤ Name and create (the name lives in the basics section).
    const name = `NATS wizard ${width}`;
    await basics.locator('input[name="name"]').fill(name);
    await fits(`wizard confirm ${width}`);
    await shot(`wizard-5-confirm-${width}`);
    await reachable(createButton, `${width} confirm`);
    await createButton.click();
    await page.waitForURL(url => !url.pathname.endsWith('/new') && url.pathname.includes('/deployments/'));
    const deploymentId = new URL(page.url()).pathname.split('/').filter(Boolean).at(-1);
    const saved = await get(`${root}/deployments/${deploymentId}`);
    const savedSpec = parse(saved.deploy_spec);
    check(savedSpec.failover?.mode === 'alert', `${width}: failure handling is saved in the deployment spec`);
    const bound = saved.blueprint_binding.roles.nats.map(item => item.hostId);
    check(saved.name === name && saved.target_kind === 'blueprint' && saved.mode === 'docker', `${width}: the deployment is created as a docker blueprint deployment`);
    check(JSON.stringify(savedSpec.roles.nats.hosts.tags) === '["nats-ready"]' && savedSpec.values.client_port === clientPort, `${width}: the stored spec keeps the tag selector and the YAML-edited value`);
    check(bound[0] === moved && bound[1] === before[0] && bound[2] === before[2], `${width}: the stored binding is the previewed placement with the manual change`);
    check(saved.blueprint_binding.roles.nats.every(item => item.address === hosts.find(h => h.id === item.hostId).address), `${width}: stored addresses come from the hosts`);

    // Detail: role → instance → host topology.
    await page.getByText('目标 · Docker · 3 台主机', { exact: true }).waitFor();
    check(true, `${width}: the detail header names the driver and the number of bound hosts`);
    const role = page.locator('[data-topology-role="nats"]');
    await role.waitFor();
    for (const [index, hostId] of bound.entries()) {
      const instance = role.locator(`[data-topology-instance="nats-${index}"]`);
      const hostName = hosts.find(h => h.id === hostId).name;
      // Host names arrive with their own request after the deployment renders.
      await instance.getByText(hostName).waitFor();
      check((await instance.innerText()).includes(hostName), `${width}: topology shows nats-${index} on its host`);
    }
    await page.waitForLoadState('networkidle');
    await fits(`detail topology ${width}`);
    await role.scrollIntoViewIfNeeded();
    await shot(`detail-topology-${width}`);

    // Simulate an already deployed view without running Docker. The form still
    // saves to the real API; only the GET presentation's revision marker is stubbed.
    const detailPath = `${origin}/api${root}/deployments/${deploymentId}`;
    await page.route(detailPath, async route => {
      const response = await route.fetch();
      if (route.request().method() !== 'GET') return route.fulfill({ response });
      await route.fulfill({ response, json: { ...await response.json(), current_revision: '999' } });
    });
    await page.reload();
    await page.getByRole('button', { name: '编辑部署规格', exact: true }).click();
    await failureMode.waitFor();
    // The detail page has its own read-only sections; inspect only the spec editor that holds failure handling.
    const specEditor = page.locator('[data-scroll-form-sections]').filter({ has: failureMode }).last();
    check(await specEditor.getByRole('region', { name: '部署目标', exact: true }).count() === 0 && await specEditor.locator('section[aria-label="角色与主机"], section[aria-label="放置预览"]').count() === 0
      && await specEditor.getByRole('region', { name: '模板参数', exact: true }).filter({ has: failureMode }).isVisible() && await failureMode.isEnabled(), `${width}: frozen topology keeps only parameters and editable failure handling`);
    await failureMode.selectOption('off');
    await fits(`frozen failover ${width}`);
    await shot(`wizard-frozen-failover-${width}`);
    const save = page.getByRole('button', { name: '保存', exact: true });
    await save.scrollIntoViewIfNeeded();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await reachable(save, `${width} frozen failover`);
    await page.screenshot({ path: join(output, `wizard-frozen-save-${width}.png`) });
    await reachable(save, `${width} frozen failover after capture`);
    const savedPolicy = page.waitForResponse(response => response.url() === detailPath && response.request().method() === 'PATCH');
    await save.click();
    check((await savedPolicy).ok(), `${width}: frozen failure policy saves without a topology rejection`);
    await eventually(async () => parse((await get(`${root}/deployments/${deploymentId}`)).deploy_spec).failover?.mode === 'off', 'failure policy persisted');
    await page.unrouteAll({ behavior: 'wait' });

    // Task 9 presentation fixtures only: no actual failure, fence or acknowledgement
    // is written to the isolated DB. Real failover remains Linux/Docker CI coverage.
    let acknowledged = false;
    await page.route(detailPath, async route => {
      const response = await route.fetch();
      await route.fulfill({ response, json: { ...await response.json(), failover: {
        mode: 'auto', host_down_after: 90,
        fences: [{ instance: 'nats-0', host_id: saved.blueprint_binding.roles.nats[0].hostId, created_at: 1000 }],
        attention: acknowledged ? [] : [{ instance: 'nats-1', since: 2000 }],
      } } });
    });
    await page.route(`${detailPath}/events**`, route => route.fulfill({ json: {
      items: [{ id: '701', kind: 'replaceFailed', instance: 'nats-1', host_id: bound[1], host_name: 'test-host', revision_id: '999', created_at: 3000,
        message: '替换失败：等待人工排查。' + '超长消息必须换行'.repeat(100), detail: { operation_id: '700' } }], total: 1, page: 1, page_size: 20,
    } }));
    await page.route(`${detailPath}/instances/nats/1/acknowledgements`, async route => {
      check(route.request().method() === 'POST' && route.request().postDataJSON().version === saved.version + 1, `${width}: acknowledgement fixture receives current version`);
      acknowledged = true; await route.fulfill({ json: {} });
    });
    await page.reload();
    const attention = page.getByRole('region', { name: '需人工处理', exact: true });
    await attention.waitFor();
    const acknowledge = attention.getByRole('button', { name: '确认', exact: true });
    await acknowledge.scrollIntoViewIfNeeded();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await reachable(acknowledge, `${width} failover acknowledge`);
    await fits(`failover attention ${width}`);
    await page.screenshot({ path: join(output, `failover-attention-${width}.png`) });
    const fencedRow = page.locator('[data-topology-instance="nats-0"]');
    check(await fencedRow.getByText('已隔离', { exact: true }).isVisible(), `${width}: fence badge matches host and instance`);
    check(await page.locator('[data-topology-instance="nats-1"]').getByText('需人工处理', { exact: true }).isVisible(), `${width}: attention badge shown`);
    await fencedRow.scrollIntoViewIfNeeded();
    await page.screenshot({ path: join(output, `failover-topology-${width}.png`) });
    const timeline = page.getByRole('region', { name: '自动动作', exact: true });
    await timeline.getByText('test-host', { exact: true }).waitFor();
    await timeline.scrollIntoViewIfNeeded();
    await fits(`failover long timeline ${width}`);
    await shot(`failover-timeline-${width}`);
    check(await timeline.getByRole('link', { name: '修订 999', exact: true }).count() === 1, `${width}: event links to its revision`);
    await acknowledge.click();
    await attention.waitFor({ state: 'detached' });
    check(acknowledged, `${width}: acknowledged presentation refreshes`);
    await page.unrouteAll({ behavior: 'wait' });
  }
  const created = (await get(`${root}/deployments?page_size=50`)).items;
  check(created.length === 4 && created.every(row => row.revision_number === 0), 'four deployments created, none of them deployed');
  assert.deepEqual(errors, [], 'page errors');
  check(true, 'no page errors');
  passed = true;
  console.log(`PASS: ${checks.length} checks; screenshots in ${output}`);
} finally {
  if (!passed) console.error((await readFile(join(work, 'server.log'), 'utf8').catch(() => '')).slice(-4000));
  await browser?.close();
  if (server.exitCode === null) { server.kill('SIGTERM'); await new Promise(r => server.once('exit', r)); }
  fixture.close();
  await log.close();
  if (passed) await rm(work, { recursive: true, force: true });
}
