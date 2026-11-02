/** Real application in headless Chromium, with fixture API responses only.
 * This verifies browser interactions and layout; it does not contact a provider,
 * validate private credentials, run act, or prove backend persistence.
 */
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createServer } from 'vite';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');

const output = resolve('data/maintenance/contribution-ui');
await mkdir(output, { recursive: true });
const server = process.env.CONTRIBUTION_UI_URL ? null : await createServer({ server: { host: '127.0.0.1', port: 0, strictPort: false, open: false } });
if (server) await server.listen();
const origin = process.env.CONTRIBUTION_UI_URL || `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
const errors = [], consoleErrors = [], unexpected = [], requests = [], checks = [];
let allowPipeline = true;
let config = { name: 'Fixture insight', description: '', enabled: true, update_mode: 'scheduled', schedule: '0 */4 * * *', timezone: '', next_run_at: 1800000000000, repositories: [{connection_id:'11',connection_name:'Fixture connection',repository:'team/repository',pipeline_id:''}], full_history: false, initial_days: 90, reconcile_minutes: 1440, aliases: [{email:'alice@example.invalid',nickname:'Alice',contributor_id:'100'}] };
let created;
let version = 1, retries = 0;
const now = Date.now(), iso = offset => new Date(now - offset * 86400000).toISOString().slice(0, 10);
const source = { repository: 'team/repository', state: 'partial', synced_at: now, coverage_start: null, last_error: 'Fixture: one branch awaits complete diff', job: { state: 'queued', generation: 2, completed_generation: 1 } };
const series = [
  { id: 'd1:A', period: iso(3), date: iso(3), nickname: 'Alice', name: 'Alice', email: 'alice@example.invalid', projects: 1, commits: 2, additions: 20, deletions: 5, effectives: 15 },
  { id: 'd1:B', period: iso(3), date: iso(3), nickname: 'Bob', name: 'Bob', email: 'bob@example.invalid', projects: 1, commits: 1, additions: 12, deletions: 2, effectives: 10 },
  { id: 'd2:A', period: iso(1), date: iso(1), nickname: 'Alice', name: 'Alice', email: 'alice@example.invalid', projects: 1, commits: 3, additions: 40, deletions: 7, effectives: 33 },
];
const coverage = { complete: false, start: null, sources: [source], timezone: 'UTC' };
const envelope = (items, params, metadata = {}) => {
  const page = Number(params.get('page') || 1), size = Number(params.get('page_size') || 20);
  return { items: items.slice((page - 1) * size, page * size), total: items.length, page, page_size: size, ...metadata };
};
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1280, height: 800 } });
  await context.route('**/api/**', async route => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    requests.push({ path, query: url.search, method: request.method(), body: request.postDataJSON() });
    const json = body => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    if (path === '/api/setup') return json({ required: false, has_users: true, registration_enabled: false });
    if (path === '/api/me') return json({ user: { id: '1', name: 'Fixture owner', email: 'owner@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Fixture workspace', slug: 'fixture', role_name: '拥有者', permissions: ['repositories.read', 'repositories.update', 'credentials.read', 'code-insights.read', 'code-insights.create', 'code-insights.update', 'code-insights.delete', 'code-insights.sync', ...(allowPipeline ? ['pipelines.read', 'pipelines.run'] : [])] }] });
    if (path === '/api/workspaces/2/preferences') return json({ default_page_size: 20, dictionaries: {} });
    if (path === '/api/workspaces/2/code-connections/11') return json({ id: '11', name: 'Fixture connection', provider: 'github', state: 'connected', credential_id: '12', version: 1 });
    if (path === '/api/workspaces/2/code-insights') {
      if (request.method() === 'POST') { created = request.postDataJSON(); return json({id:'32',version:1}); }
      return json(envelope([{id:'31',...config}, ...(created ? [{id:'32',...created}] : [])],url.searchParams));
    }
    if (path === '/api/workspaces/2/code-insights/31') {
      if (request.method() === 'PATCH') { const body=request.postDataJSON(); assert.equal(body.version,version); config=body;version++;return json({id:'31',version}); }
      return json({id:'31',...config,version,coverage_start:null,last_error:'',sources:[source]});
    }
    if (path === '/api/workspaces/2/code-insights/32') return json({id:'32',...created,version:1,sources:[]});
    if (path === '/api/workspaces/2/code-connections') return json(envelope([{id:'11',name:'Fixture connection',provider:'github',state:'connected'},{id:'12',name:'Generic Git',provider:'git',state:'connected'}],url.searchParams));
    if (path === '/api/workspaces/2/code-connections/11/repositories') return json({items:[{name:'repository',full_name:'team/repository',url:'https://example.invalid/team/repository.git',default_branch:'main'}],has_more:false});
    if (path === '/api/workspaces/2/code-insights/31/sync' && request.method() === 'POST') { retries++; return json({ id: '31', queued: true }); }
    if (path === '/api/workspaces/2/pipelines') { assert(allowPipeline, 'limited role must not request pipelines'); return json(envelope([{ id: '21', name: 'Fixture analysis workflow' }], url.searchParams)); }
    if (path.includes('/code-insights/') && path.includes('/reports/')) {
      assert.equal(url.searchParams.has('connection_id'), false);
      const kind = path.split('/').at(-1);
      let items = series;
      if (kind === 'overview') items = [{ id: 'total', projects: 1, commits: 6, additions: 72, deletions: 14, effectives: 58 }];
      if (kind === 'contributors' || kind === 'ranking') items = [series[0], series[1]];
      if (kind === 'branches') items = [{ id: 'branch', repository: 'team/repository', branchName: 'main', present: true, nicknames: ['Alice', 'Bob'], commits: 6, additions: 72, deletions: 14, effectives: 58 }];
      if (kind === 'commits') items = [{ id: 'commit', repository: 'team/repository', branches: ['main', 'release'], commitHash: 'a'.repeat(40), commitUrl: 'https://example.invalid/team/repository/commit/' + 'a'.repeat(40), message: 'feat: browser fixture', messageType: 'feat', isMerge: false, nickname: 'Alice', authorName: 'Alice', authorEmail: 'alice@example.invalid', date: now, committerDate: now, additions: 20, deletions: 5, effectives: 15, languageStats: { TypeScript: { additions: 20, deletions: 5 } } }];
      return json(envelope(items, url.searchParams, { coverage, ...(['heatmap', 'period'].includes(kind) ? { series } : {}) }));
    }
    unexpected.push(path);
    return route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: 'Unspecified browser fixture route' }) });
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
  page.setDefaultTimeout(15000);
  const connectionPath = `${origin}/workspaces/2/code-connections/11`;
  await page.goto(connectionPath);
  await page.getByRole('tab', { name: '代码洞察', exact: true }).click();
  await page.getByRole('region',{name:'相关代码洞察'}).getByRole('link',{name:'查看',exact:true}).click();
  const insightPath = origin + '/workspaces/2/code-insights/31';
  await page.getByText('Fixture: one branch awaits complete diff', { exact: false }).waitFor();
  await page.getByRole('region', { name: '六个周期贡献排名', exact: true }).waitFor();
  assert.equal((await page.locator('body').innerText()).includes('Invalid Date'), false);
  assert.equal(await page.getByText('覆盖起点：1970', { exact: false }).count(), 0, 'unknown coverage must not show epoch');
  checks.push('standalone insight through related connection list, six ranking tables, partial source progress and null coverage');

  await page.getByRole('tab',{name:'配置与同步',exact:true}).click();
  await page.getByRole('link',{name:'编辑',exact:true}).click();
  assert.equal(await page.getByLabel('Cron 表达式',{exact:true}).inputValue(),'0 */4 * * *');
  assert.equal(await page.getByLabel('时区',{exact:true}).getAttribute('placeholder'),'UTC');
  assert.equal(await page.getByLabel('分支头核对间隔（分钟）',{exact:false}).count(),0);
  await page.locator('select[name="update_mode"]').selectOption('event');
  assert.equal(await page.getByLabel('Cron 表达式',{exact:true}).count(),0);
  await page.getByLabel('作者昵称映射',{exact:true}).fill('invalid-json');
  await page.getByRole('alert').filter({hasText:'JSON'}).waitFor();
  assert(await page.getByRole('button',{name:'保存',exact:true}).isDisabled());
  await page.getByLabel('作者昵称映射',{exact:true}).fill('[{"email":"alice@example.invalid","nickname":"小艾","contributor_id":"100"}]');
  await page.getByLabel('仓库连接',{exact:true}).selectOption('12');
  await page.getByLabel('Git 仓库地址',{exact:true}).fill('git@git.example.invalid:team/second.git');
  await page.getByRole('button',{name:'加入洞察',exact:true}).click();
  await page.getByLabel('分析流水线 git@git.example.invalid:team/second.git',{exact:true}).selectOption('21');
  await page.getByLabel('初始化最近天数',{exact:false}).fill('120');
  await page.getByLabel('分支头核对间隔（分钟）',{exact:false}).fill('60');
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await page.waitForURL(url=>url.pathname==='/zh-CN/workspaces/2/code-insights');
  assert.equal(config.initial_days,120);assert.equal(config.reconcile_minutes,60);
  assert.equal(config.update_mode,'event');
  assert.equal(config.repositories.length,2);assert.equal(config.repositories[1].pipeline_id,'21');
  assert.equal(config.aliases[0].nickname,'小艾');assert.equal(config.aliases[0].contributor_id,'100');
  await page.goto(insightPath);
  await page.getByRole('button',{name:'重新核对 / 重试同步',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('button[disabled][aria-label="重新核对 / 重试同步"]'));
  assert.equal(retries,1);
  checks.push('shared edit form validates aliases, preserves contributor IDs, saves memberships/pipeline and optimistic version, retries sync');

  await page.goto(origin+'/workspaces/2/code-insights/new');
  assert.equal(await page.locator('select[name="update_mode"]').inputValue(),'scheduled');
  assert.equal(await page.getByLabel('Cron 表达式',{exact:true}).inputValue(),'0 */4 * * *');
  await page.getByLabel('间隔小时',{exact:true}).fill('6');
  await page.getByLabel('时区',{exact:true}).fill('Asia/Singapore');
  await page.getByLabel('洞察名称',{exact:false}).fill('Independent insight');
  await page.getByLabel('仓库连接',{exact:true}).selectOption('11');
  await page.getByLabel('添加洞察仓库',{exact:true}).selectOption('https://example.invalid/team/repository.git');
  await page.getByRole('button',{name:'加入洞察',exact:true}).click();
  await page.getByLabel('作者昵称映射',{exact:true}).fill('[{"email":"alice@example.invalid","nickname":"Separate Alice"}]');
  await page.getByRole('button',{name:'创建',exact:true}).click();
  await page.waitForURL(url=>url.pathname==='/zh-CN/workspaces/2/code-insights');
  assert.equal(created.repositories[0].repository,'team/repository');
  assert.equal(created.update_mode,'scheduled');assert.equal(created.schedule,'0 */6 * * *');assert.equal(created.timezone,'Asia/Singapore');
  assert.equal(created.aliases[0].nickname,'Separate Alice');assert.equal(config.aliases[0].nickname,'小艾');
  checks.push('creates independent insight through shared form, same repository selected twice with different mappings');
  await page.goto(insightPath);

  const dimensions = [['contributors', '贡献者'], ['ranking', '贡献排名'], ['branches', '分支'], ['commits', '提交明细'], ['heatmap', '每日热力数据'], ['period', '周期统计'], ['overview', '总览']];
  const report = page.getByRole('region', { name: '贡献分析报告', exact: true });
  await report.locator('tbody tr').first().waitFor();
  for (const [kind, label] of dimensions) {
    await Promise.all([
      page.waitForResponse(response => new URL(response.url()).pathname.endsWith(`/reports/${kind}`) && response.ok()),
      page.getByRole('button', { name: label, exact: true }).click(),
    ]);
    await page.waitForURL(url => url.searchParams.get('analysis_report') === (kind === 'overview' ? null : kind));
    await page.waitForLoadState('networkidle');
    await report.locator('tbody tr').first().waitFor();
    const visibleRow = report.locator('tbody tr').first();
    await visibleRow.scrollIntoViewIfNeeded();
    assert(await visibleRow.evaluate(element => { let node = element.parentElement; const row = element.getBoundingClientRect(); while (node) { const style = getComputedStyle(node); if (['auto', 'scroll', 'hidden'].includes(style.overflowY)) { const bounds = node.getBoundingClientRect(); if (Math.min(bounds.bottom, row.bottom) - Math.max(bounds.top, row.top) < 10) return false; } node = node.parentElement; } return true; }), `${kind}: rows must remain visible below filters and charts`);
    if (kind === 'commits') {
      const link = page.getByRole('link', { name: '在源平台查看提交 ' + 'a'.repeat(40), exact: true });
      assert.equal(await link.getAttribute('href'), 'https://example.invalid/team/repository/commit/' + 'a'.repeat(40));
      await report.getByRole('cell', { name: 'main\nrelease', exact: true }).waitFor();
    }
    if (kind === 'heatmap') { assert((await report.locator('svg rect').count()) > 0); }
    if (kind === 'period') { assert.equal(await report.locator('[data-author]').count(), 2); await report.locator('svg[role="img"]').scrollIntoViewIfNeeded(); await page.screenshot({ path: resolve(output, 'desktop-trend.png') }); }
  }
  checks.push('all seven real shared report views, source commit link, branch membership, heatmap and per-author trend');

  await page.getByRole('button', { name: '每日热力数据', exact: true }).click();
  const shortcuts = page.getByRole('group', { name: '热力图提交类型', exact: true });
  for (const [label, merge, messageType] of [['全部提交', '0,1', null], ['修复 fix', '0', 'fix'], ['功能 feat', '0', 'feat'], ['合并 merge', '1', null]]) {
    const response = page.waitForResponse(value => { const url = new URL(value.url()); return url.pathname.endsWith('/heatmap') && url.searchParams.get('isMerge') === merge && url.searchParams.get('messageType') === messageType; });
    await shortcuts.getByRole('button', { name: label, exact: true }).click(); await response;
  }
  for (const label of ['今天', '昨天', '本周', '上周', '上上周', '本月', '上月', '上上月', '全部', '默认（已采集范围）']) {
    await page.getByRole('group', { name: '日期快捷条件（UTC）', exact: true }).getByRole('button', { name: label, exact: true }).click();
    await page.waitForFunction(expected => { const params = new URLSearchParams(location.search); return expected ? params.has('since') && params.has('until') : !params.has('since') && !params.has('until'); }, !['全部', '默认（已采集范围）'].includes(label));
  }
  checks.push('all date presets commit URL filters; heatmap all/fix/feat/merge request semantics');

  // URL updates precede the async report reload; wait for the final report
  // before scrolling a chart that the pending reload can replace.
  await page.waitForLoadState('networkidle');
  await report.locator('svg[role="img"]').waitFor();

  for (const viewport of [{ width: 1280, height: 800 }, { width: 390, height: 844 }]) {
    await page.setViewportSize(viewport);
    await report.locator('svg[role="img"]').evaluate(node => node.scrollIntoView({ block: 'center' }));
    const width = await page.evaluate(() => ({ document: document.documentElement.scrollWidth, viewport: innerWidth }));
    assert(width.document <= width.viewport + 1, `page overflow at ${viewport.width}: ${JSON.stringify(width)}`);
    await page.screenshot({ path: resolve(output, `${viewport.width}-heatmap.png`) });
    await page.goto(insightPath+'/edit');
    await page.getByLabel('作者昵称映射',{exact:true}).waitFor();
    const formWidth=await page.locator('form').evaluate(node=>node.getBoundingClientRect().width);
    assert(formWidth<=viewport.width+1,'shared edit form fits viewport');
    await page.goto(insightPath+'?analysis_report=heatmap');
    await report.locator('svg[role="img"]').waitFor();
  }
  checks.push('1280x800 and 390x844 viewport, document overflow, shared edit form bounds and screenshots');

  const insightCatalog = JSON.parse(await readFile(new URL('../src/locales/insight-domain.json', import.meta.url), 'utf8'));
  const localeOrder = ['en-US', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'];
  const insightLabel = (key, locale) => locale === 'zh-CN' ? key : insightCatalog[key][localeOrder.indexOf(locale)];
  await page.setViewportSize({ width: 1366, height: 900 });
  await page.goto(`${origin}/zh-CN/workspaces/2/code-insights/31?analysis_report=period`);
  for (const locale of ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY']) {
    await page.locator('select').filter({ has: page.locator('option[value="en-US"]') }).selectOption(locale);
    await page.waitForURL(url => url.pathname.startsWith(`/${locale}/`));
    const localizedReport = page.getByRole('region', { name: insightLabel('贡献分析报告', locale), exact: true });
    await localizedReport.getByRole('button', { name: insightLabel('周期统计', locale), exact: true }).waitFor();
    await localizedReport.locator('svg[role="img"]').waitFor();
    assert.equal(await localizedReport.locator('[data-author="Alice"]').count(), 1);
    assert.equal(await localizedReport.getByRole('group', { name: insightLabel('日期快捷条件（UTC）', locale), exact: true }).count(), 1);
    assert.equal(await localizedReport.getByRole('group', { name: insightLabel('日期快捷条件（UTC）', locale), exact: true }).getByRole('button', { name: insightLabel('全部', locale), exact: true }).count(), 1);
  }
  await page.locator('select').filter({ has: page.locator('option[value="en-US"]') }).selectOption('zh-CN');
  await page.waitForURL(url => url.pathname.startsWith('/zh-CN/'));
  checks.push('all six locales translate live report dimensions, date presets and chart labels while preserving author identities');

  allowPipeline = false;
  const pipelineReadsBefore = requests.filter(request => request.path.endsWith('/pipelines')).length;
  await page.goto(insightPath+'/edit');
  await page.getByLabel('作者昵称映射',{exact:true}).waitFor();
  assert.equal(await page.getByLabel('分析流水线 team/repository',{exact:true}).count(),0);
  assert.equal(requests.filter(request=>request.path.endsWith('/pipelines')).length,pipelineReadsBefore);
  checks.push('limited role keeps saved memberships readable and never fetches pipeline options');

  assert.deepEqual(unexpected, []); assert.deepEqual(errors, []); assert.deepEqual(consoleErrors, []);
  await writeFile(resolve(output, 'result.json'), JSON.stringify({ scope: 'Real Chromium application, fixture APIs only; no private provider or backend persistence claim', checks, pageErrors: errors, consoleErrors, screenshots: ['desktop-trend.png', '1280-heatmap.png', '390-heatmap.png'] }, null, 2));
  for (const check of checks) console.log('PASS:', check);
  console.log('PASS: zero runtime page errors, zero browser console errors. Fixture API browser scope only. Artifacts:', output);
} catch (error) {
  for (const context of browser?.contexts() ?? []) for (const page of context.pages()) {
    console.error('PAGE', page.url(), await page.locator('body').innerText().catch(() => ''));
    await page.screenshot({ path: resolve(output, 'failure.png') }).catch(() => {});
  }
  console.error({ errors, consoleErrors, unexpected });
  throw error;
} finally {
  if (browser) await browser.close();
  if (server) {
    server.httpServer.closeAllConnections();
    await server.close();
  }
}
