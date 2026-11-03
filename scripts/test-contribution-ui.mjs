/** Real application in headless Chromium, with fixture API responses only.
 * This verifies browser interactions and layout; it does not contact a provider,
 * validate private credentials, run act, or prove backend persistence.
 */
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createServer } from 'vite';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');

async function chooseNative(control, value) {
  await control.waitFor();
  await control.page().waitForFunction(({ element, expected }) => !element.disabled && [...element.options].some(option => option.value === expected), { element: await control.elementHandle(), expected: value });
  const index = await control.evaluate((element, expected) => [...element.options].findIndex(option => option.value === expected), value);
  await control.click();
  await control.press('Home');
  for (let i = 0; i < index; i++) await control.press('ArrowDown');
  await control.press('Enter');
  assert.equal(await control.inputValue(), value, 'Native selection must persist');
}

async function chooseOption(page, label, option) {
  await page.getByRole('combobox', { name: label }).click();
  await page.getByRole('option', { name: option, exact: true }).click();
}

const output = resolve('data/maintenance/contribution-ui');
await mkdir(output, { recursive: true });
const server = process.env.CONTRIBUTION_UI_URL ? null : await createServer({ server: { host: '127.0.0.1', port: 0, strictPort: false, open: false } });
if (server) await server.listen();
const origin = process.env.CONTRIBUTION_UI_URL || `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
const errors = [], consoleErrors = [], unexpected = [], requests = [], checks = [];
let allowPipeline = true;
let allowRepositoryRead = true;
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
const heatmapSeries = [...new Set(series.map(row => row.date))].map(date => {
  const authors = series.filter(row => row.date === date);
  return { id: date, date, period: date, contributors: authors.length, authors,
    ...Object.fromEntries(['commits', 'additions', 'deletions', 'effectives'].map(metric => [metric, authors.reduce((sum, row) => sum + row[metric], 0)])) };
});
const envelope = (items, params, metadata = {}) => {
  const page = Number(params.get('page') || 1), size = Number(params.get('page_size') || 20);
  return { items: items.slice((page - 1) * size, page * size), total: items.length, page, page_size: size, ...metadata };
};
try {
  const chrome = process.env.PLAYWRIGHT_CHROME_PATH || 'C:/Program Files/Google/Chrome/Application/chrome.exe';
  browser = await chromium.launch({ headless: true, ...(existsSync(chrome) ? { executablePath: chrome } : {}) });
  const context = await browser.newContext({ locale: 'zh-CN', viewport: { width: 1280, height: 800 } });
  await context.route('**/api/**', async route => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    if(!path.startsWith('/api/'))return route.continue();
    requests.push({ path, query: url.search, method: request.method(), body: request.postDataJSON() });
    const json = body => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    if (path === '/api/setup') return json({ required: false, has_users: true, registration_enabled: false });
    if (path === '/api/me') return json({ user: { id: '1', name: 'Fixture owner', email: 'owner@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Fixture workspace', slug: 'fixture', role_name: '拥有者', permissions: [...(allowRepositoryRead?['repositories.read']:[]), 'repositories.update', 'credentials.read', 'code-insights.read', 'code-insights.create', 'code-insights.update', 'code-insights.delete', 'code-insights.sync', ...(allowPipeline ? ['pipelines.read', 'pipelines.run'] : [])] }] });
    if (path === '/api/workspaces/2/preferences') return json({ default_page_size: 20, dictionaries: {} });
    if (path === '/api/workspaces/2/repository-events') return json(envelope([],url.searchParams));
    if (path === '/api/workspaces/2/code-connections/11') return json({ id: '11', name: 'Fixture connection', provider: 'github', state: 'connected', credential_id: '12', version: 1 });
    if (path === '/api/workspaces/2/code-insights') {
      if (request.method() === 'POST') { created = request.postDataJSON(); return json({id:'32',version:1}); }
      return json(envelope([{id:'31',...config}, ...(created ? [{id:'32',...created}] : [])],url.searchParams));
    }
    if (path === '/api/workspaces/2/code-insights/31') {
      if (request.method() === 'PATCH') { const body=request.postDataJSON(); assert.equal(body.version,version); config=body;version++;return json({id:'31',version}); }
      return json({id:'31',...config,version,coverage_start:null,last_error:'',sources:[source]});
    }
    if (path === '/api/workspaces/2/code-insights/32') {
      if(request.method()==='PATCH')created={...created,...request.postDataJSON()};
      return json({id:'32',...created,version:1,sources:[]});
    }
    if (path === '/api/workspaces/2/code-connections') return json(envelope([{id:'11',name:'Fixture connection',provider:'github',state:'connected'},{id:'12',name:'Generic Git',provider:'git',state:'connected'}],url.searchParams));
    if (path === '/api/workspaces/2/code-connections/11/repositories') return json({items:[{name:'repository',full_name:'team/repository',url:'https://example.invalid/team/repository.git',default_branch:'main'}],has_more:false});
    if (path === '/api/workspaces/2/code-insights/31/sync' && request.method() === 'POST') { retries++; return json({ id: '31', queued: true }); }
    if (path === '/api/workspaces/2/pipelines') { assert.fail('Insight collection must never request business pipelines'); }
    if (path.includes('/code-insights/') && path.includes('/reports/')) {
      assert.equal(url.searchParams.has('connection_id'), false);
      const kind = path.split('/').at(-1);
      let items = series;
      if (kind === 'overview') items = [{ id: 'total', projects: 1, commits: 6, additions: 72, deletions: 14, effectives: 58 }];
      if (kind === 'contributors' || kind === 'ranking') items = [series[0], series[1]];
      if (kind === 'repositories') items = [{ id: 'repository', repository: 'team/repository', nicknames: ['Alice', 'Bob'], commits: 6, additions: 72, deletions: 14, effectives: 58 }];
      if (kind === 'branches') items = [{ id: 'branch', repository: 'team/repository', branchName: 'main', present: true, nicknames: ['Alice', 'Bob'], commits: 6, additions: 72, deletions: 14, effectives: 58 }];
      if (kind === 'commits') items = [{ id: 'commit', repository: 'team/repository', branches: ['main', 'release'], commitHash: 'a'.repeat(40), commitUrl: 'https://example.invalid/team/repository/commit/' + 'a'.repeat(40), message: 'feat: browser fixture', messageType: 'feat', isMerge: false, nickname: 'Alice', authorName: 'Alice', authorEmail: 'alice@example.invalid', date: now, committerDate: now, additions: 20, deletions: 5, effectives: 15, languageStats: { TypeScript: { additions: 20, deletions: 5 } } }];
      return json(envelope(items, url.searchParams, { coverage, ...(['heatmap', 'period'].includes(kind) ? { series: kind === 'heatmap' ? heatmapSeries : series } : {}) }));
    }
    unexpected.push(path);
    return route.fulfill({ status: 404, contentType: 'application/json', body: JSON.stringify({ error: 'Unspecified browser fixture route' }) });
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') consoleErrors.push(message.text()); });
  page.setDefaultTimeout(15000);
  page.setDefaultNavigationTimeout(60000);
  const connectionPath = `${origin}/workspaces/2/code-connections/11`;
  await page.goto(connectionPath, {waitUntil:'domcontentloaded'});
  console.log('Contribution UI: application document loaded');
  const eventsLink=page.getByRole('link',{name:'仓库事件',exact:true});
  await eventsLink.waitFor();
  assert.equal(await eventsLink.evaluate(element=>{const insights=[...document.querySelectorAll('a')].find(link=>link.textContent.trim()==='代码洞察');return !!insights&&!!(element.compareDocumentPosition(insights)&Node.DOCUMENT_POSITION_FOLLOWING);}),true,'Repository events appears before code insights in repository navigation');
  const eventsRead=page.waitForRequest(request=>new URL(request.url()).pathname==='/api/workspaces/2/repository-events');
  await eventsLink.click();
  const eventsRequest=await eventsRead;
  assert.equal(new URL(eventsRequest.url()).searchParams.has('connection_id'),false,'Workspace events are not silently scoped to one connection');
  await page.goto(connectionPath);
  await page.getByRole('tab', { name: '代码洞察', exact: true }).click();
  await page.getByRole('region',{name:'相关代码洞察'}).getByRole('link',{name:'查看',exact:true}).click();
  const insightPath = origin + '/zh-CN/workspaces/2/code-insights/31';
  const sourceDetails = page.getByText('各仓库覆盖与任务状态', { exact: true });
  await sourceDetails.waitFor();
  assert.equal(await sourceDetails.evaluate(element => element.parentElement.open), false, 'repository task details are collapsed initially');
  await sourceDetails.click();
  await page.getByText('Fixture: one branch awaits complete diff', { exact: false }).waitFor();
  await page.getByRole('region', { name: '六个周期贡献排名', exact: true }).waitFor();
  assert.equal((await page.locator('body').innerText()).includes('Invalid Date'), false);
  assert.equal(await page.getByText('覆盖起点：1970', { exact: false }).count(), 0, 'unknown coverage must not show epoch');
  const report = page.getByRole('region', { name: '贡献分析报告', exact: true });
  const sharedForm = report.getByRole('form', { name: '统一分析筛选', exact: true });
  assert.equal(await report.locator('form').count(), 1, 'Overview has exactly one shared filter form');
  assert.equal(await report.getByRole('button', { name: '搜索', exact: true }).count(), 1);
  assert.equal(await report.locator('table').count(), 0, 'Overview presents charts rather than stacked CRUD tables');
  assert.equal(await page.getByRole('region', { name: '六个周期贡献排名', exact: true }).locator(':scope > div > section').count(), 6);
  await page.waitForLoadState('networkidle');
  await page.evaluate(()=>document.fonts.ready);
  for(const width of [1366,390]) {
    await page.setViewportSize({width,height:900});
    await page.evaluate(()=>document.querySelectorAll('*').forEach(element=>{if(element.scrollTop)element.scrollTop=0;}));
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`Overview fits ${width}px`);
    await page.screenshot({path:resolve(output,`overview-top-${width}.png`)});
    await page.getByRole('region',{name:'六个周期贡献排名',exact:true}).locator(':scope > div > section').first().evaluate(element=>element.scrollIntoView({block:'start'}));
    await page.screenshot({path:resolve(output,`overview-cards-${width}.png`)});
  }
  await page.setViewportSize({width:1280,height:800});
  checks.push('standalone insight through related connection list, six compact period cards, one shared filter, partial source progress and null coverage');

  await sharedForm.getByLabel('仓库（逗号分隔）', { exact: true }).fill('team/repository');
  await sharedForm.getByLabel('作者昵称（逗号分隔）', { exact: true }).fill('Alice');
  await sharedForm.getByLabel('起始日期（UTC）', { exact: true }).fill(iso(30));
  await sharedForm.getByLabel('截止日期（UTC）', { exact: true }).fill(iso(0));
  await sharedForm.getByRole('button', { name: '高级筛选', exact: true }).click();
  await sharedForm.getByLabel('分支（逗号分隔）', { exact: true }).fill('main');
  await sharedForm.getByLabel('提交类型（逗号分隔）', { exact: true }).fill('feat');
  await sharedForm.getByLabel('有效变更下限', { exact: true }).fill('1');
  await sharedForm.getByLabel('有效变更上限', { exact: true }).fill('500');
  await sharedForm.getByLabel('完整提交 SHA', { exact: true }).fill('a'.repeat(40));
  await chooseOption(page, '分支历史', '当前可达提交');
  await chooseOption(page, '合并提交', '包含合并');
  const filteredStart = requests.length;
  await sharedForm.getByRole('button', { name: '搜索', exact: true }).click();
  await page.waitForLoadState('networkidle');
  const filtered = requests.slice(filteredStart).filter(request => request.path.includes('/reports/'));
  assert.equal(filtered.length, 13, 'One overview search loads totals and six branch/author pairs');
  for (const request of filtered) {
    const params = new URLSearchParams(request.query);
    for (const [key, expected] of Object.entries({ repos: 'team/repository', authors: 'Alice', branches: 'main', messageType: 'feat', geEffective: '1', leEffective: '500', isMerge: '0,1', history: 'current', commitHash: 'a'.repeat(40) })) assert.equal(params.get(key), expected, `${request.path}: shared ${key}`);
    if(request.path.endsWith('/overview')) { assert.equal(params.get('since'),iso(30)); assert.equal(params.get('until'),iso(0)); }
  }
  const firstCard = page.getByRole('region', { name: '六个周期贡献排名', exact: true }).locator(':scope > div > section').first();
  await firstCard.getByRole('button', { name: '贡献排名', exact: true }).click();
  await page.waitForURL(url => url.searchParams.get('analysis_report') === 'ranking');
  assert.equal(new URL(page.url()).searchParams.get('authors'), 'Alice', 'Period drilldown retains shared non-date conditions');
  assert(new URL(page.url()).searchParams.has('since'), 'Period drilldown supplies its date interval');
  await sharedForm.getByRole('button', { name: '清空', exact: true }).click();
  await report.getByRole('button', { name: '总览', exact: true }).click();
  checks.push('all thirteen overview requests share repository/author/branch/advanced filters; period drilldown preserves conditions');

  await page.getByRole('tab',{name:'配置与同步',exact:true}).click();
  await page.getByRole('link',{name:'编辑',exact:true}).waitFor();
  assert.equal(await page.getByText('各仓库覆盖与任务状态',{exact:true}).count(),0,'Settings detail excludes runtime task state');
  assert.equal(await page.getByLabel('说明',{exact:true}).count(),0,'Settings detail excludes the removed description field');
  await page.getByRole('link',{name:'编辑',exact:true}).click();
  await page.getByLabel('Cron 表达式',{exact:true}).waitFor();
  assert.equal(await page.getByText('各仓库覆盖与任务状态',{exact:true}).count(),0,'Edit form excludes runtime task state');
  assert.equal(await page.getByLabel('说明',{exact:true}).count(),0,'Edit form excludes the removed description field');
  assert.equal(await page.getByLabel('Cron 表达式',{exact:true}).inputValue(),'0 */4 * * *');
  assert.equal(await page.getByLabel('时区',{exact:true}).getAttribute('placeholder'),'UTC');
  assert.equal(await page.getByLabel('分支头核对间隔（分钟）',{exact:false}).count(),0);
  await chooseOption(page, /^更新方式/, '事件更新');
  assert.equal(await page.getByLabel('Cron 表达式',{exact:true}).count(),0);
  assert.equal(await page.getByLabel('作者昵称映射',{exact:true}).count(),0, 'Author mapping is separate from settings');
  await chooseNative(page.getByLabel('仓库连接',{exact:true}), '12');
  await page.getByLabel('Git 仓库地址',{exact:true}).fill('git@git.example.invalid:team/second.git');
  await page.getByRole('button',{name:'加入洞察',exact:true}).click();
  assert.equal(await page.getByLabel('分析流水线 git@git.example.invalid:team/second.git',{exact:true}).count(),0);
  assert.equal(await page.getByRole('button',{name:'保存',exact:true}).isEnabled(),true,'Plain Git saves with its connection and URL alone');
  await page.getByLabel('初始化最近天数',{exact:false}).fill('120');
  await page.getByLabel('分支头核对间隔（分钟）',{exact:false}).fill('60');
  await page.getByRole('button',{name:'保存',exact:true}).click();
  await page.waitForURL(url=>url.pathname==='/zh-CN/workspaces/2/code-insights');
  assert.equal(config.initial_days,120);assert.equal(config.reconcile_minutes,60);
  assert.equal(config.update_mode,'event');
  assert.equal(config.repositories.length,2);assert.equal(config.repositories[1].pipeline_id,'');
  assert.equal(config.aliases[0].nickname,'Alice');assert.equal(config.aliases[0].contributor_id,'100');
  await page.goto(insightPath+'?tab=authors');
  await page.getByLabel('统一昵称 1', { exact: true }).fill('');
  await page.waitForFunction(()=>document.querySelector('button[type="submit"]')?.disabled);
  assert(await page.getByRole('button', { name: '保存', exact: true }).isDisabled());
  await page.getByRole('alert').first().waitFor();
  await page.getByLabel('统一昵称 1', { exact: true }).fill('小艾');
  await Promise.all([page.waitForResponse(response => response.request().method() === 'GET' && new URL(response.url()).pathname.endsWith('/code-insights/31')), page.getByRole('button', { name: '保存', exact: true }).click()]);
  assert.equal(config.aliases[0].nickname,'小艾');assert.equal(config.aliases[0].contributor_id,'100');
  await page.goto(insightPath);
  await page.getByRole('button',{name:'重新核对 / 重试同步',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('button[disabled][aria-label="重新核对 / 重试同步"]'));
  assert.equal(retries,1);
  checks.push('shared settings saves plain Git without business pipelines or host permissions and preserves optimistic version; author mappings preserve contributor IDs; sync retry works');

  await page.goto(origin+'/workspaces/2/code-insights/new');
  await page.getByLabel('洞察名称',{exact:false}).waitFor();
  assert.equal(await page.getByText('各仓库覆盖与任务状态',{exact:true}).count(),0,'Create form excludes runtime task state');
  assert.equal(await page.getByLabel('说明',{exact:true}).count(),0,'Create form excludes the removed description field');
  assert.equal(await page.locator('select[name="update_mode"]').inputValue(),'scheduled');
  assert.equal(await page.getByLabel('Cron 表达式',{exact:true}).inputValue(),'0 */4 * * *');
  await page.getByLabel('间隔小时',{exact:true}).fill('6');
  await page.getByLabel('时区',{exact:true}).fill('Asia/Singapore');
  await page.getByLabel('洞察名称',{exact:false}).fill('Independent insight');
  await chooseNative(page.getByLabel('仓库连接',{exact:true}), '11');
  await page.getByRole('button',{name:'选择仓库',exact:true}).click();
  await page.getByRole('menuitemcheckbox',{name:'team/repository',exact:true}).click();
  await page.keyboard.press('Escape');
  assert.equal(await page.getByLabel('作者昵称映射',{exact:true}).count(),0);
  await page.getByRole('button',{name:'创建',exact:true}).click();
  await page.waitForURL(url=>url.pathname==='/zh-CN/workspaces/2/code-insights');
  assert.equal(created.repositories[0].repository,'team/repository');
  assert.equal(created.update_mode,'scheduled');assert.equal(created.schedule,'0 */6 * * *');assert.equal(created.timezone,'Asia/Singapore');
  assert.deepEqual(created.aliases,[]);
  await page.goto(origin+'/zh-CN/workspaces/2/code-insights/32?tab=authors');
  await page.getByRole('button',{name:'添加作者映射',exact:true}).click();
  await page.getByLabel('提交邮箱 1',{exact:true}).fill('alice@example.invalid');
  await page.getByLabel('统一昵称 1',{exact:true}).fill('Separate Alice');
  await Promise.all([page.waitForResponse(response=>response.request().method()==='GET'&&new URL(response.url()).pathname.endsWith('/code-insights/32')),page.getByRole('button',{name:'保存',exact:true}).click()]);
  assert.equal(created.aliases[0].nickname,'Separate Alice');
  assert.equal(config.aliases[0].nickname,'小艾');
  checks.push('creates independent insight through shared form, same repository selected twice with different mappings');
  await page.goto(insightPath);

  const dimensions = [['contributors', '贡献者'], ['ranking', '贡献排名'], ['repositories', '仓库'], ['branches', '分支'], ['commits', '提交明细'], ['heatmap', '每日热力数据'], ['period', '周期统计'], ['overview', '总览']];
  await report.getByRole('form', { name: '统一分析筛选', exact: true }).waitFor();
  await sharedForm.getByLabel('作者昵称（逗号分隔）', { exact: true }).fill('Alice');
  await sharedForm.getByRole('button', { name: '搜索', exact: true }).click();
  await page.waitForLoadState('networkidle');
  for (const [kind, label] of dimensions) {
    if (kind === 'branches') await sharedForm.getByRole('button', { name: '高级筛选', exact: true }).click();
    const [response] = await Promise.all([
      page.waitForResponse(response => new URL(response.url()).pathname.endsWith(`/reports/${kind}`) && response.ok()),
      page.getByRole('button', { name: label, exact: true }).click(),
    ]);
    await page.waitForURL(url => url.searchParams.get('analysis_report') === (kind === 'overview' ? null : kind));
    await page.waitForLoadState('networkidle');
    assert.equal(new URL(response.url()).searchParams.get('authors'),'Alice',`${kind}: dimension switches retain the shared author condition`);
    assert.equal(await report.locator('form').count(), 1, `${kind}: exactly one shared filter form`);
    if(kind === 'overview') { assert.equal(await report.locator('table').count(),0); continue; }
    await report.locator('tbody tr').first().waitFor();
    const visibleRow = report.locator('tbody tr').first();
    await visibleRow.scrollIntoViewIfNeeded();
    assert(await visibleRow.evaluate(element => { let node = element.parentElement; const row = element.getBoundingClientRect(); while (node) { const style = getComputedStyle(node); if (['auto', 'scroll', 'hidden'].includes(style.overflowY)) { const bounds = node.getBoundingClientRect(); if (Math.min(bounds.bottom, row.bottom) - Math.max(bounds.top, row.top) < 10) return false; } node = node.parentElement; } return true; }), `${kind}: rows must remain visible below filters and charts`);
    if (kind === 'commits') {
      const link = page.getByRole('link', { name: '在源平台查看提交 ' + 'a'.repeat(40), exact: true });
      assert.equal(await link.getAttribute('href'), 'https://example.invalid/team/repository/commit/' + 'a'.repeat(40));
      const identityCell = visibleRow.getByRole('cell').filter({ has: link });
      assert.match(await identityCell.innerText(), /team\/repository\s+main\s+release\s+a{40}/, 'repository, branches and SHA share the identity column');
      assert.match(await visibleRow.getByRole('cell').filter({ hasText: 'alice@example.invalid' }).innerText(), /Alice\s+Alice\s+alice@example\.invalid/, 'nickname, name and email share the author column');
    }
    if (kind === 'heatmap') {
      assert((await report.locator('svg rect').count()) > 0);
      assert((await report.locator('svg text').count()) >= 8, 'calendar axes show weekdays and months');
      await report.locator(`svg rect[aria-label="${iso(3)}"]`).hover();
      const tooltip = page.getByRole('tooltip'); await tooltip.waitFor();
      assert.match(await tooltip.innerText(), /Alice[\s\S]*Bob/);
      assert.match(await tooltip.innerText(), /新增行\s*32/);
      await page.mouse.move(0, 0);
    }
    if (kind === 'period') {
      assert.equal(await report.locator('[data-author]').count(), 2);
      assert((await report.locator('svg text').count()) >= 5, 'trend has visible date and value scales');
      await report.locator(`svg circle[aria-label="Alice ${iso(3)}"]`).hover();
      const tooltip = page.getByRole('tooltip'); await tooltip.waitFor();
      assert.match(await tooltip.innerText(), /Alice/);
      assert.match(await tooltip.innerText(), /新增行\s*20/);
      await page.mouse.move(0, 0);
      await report.locator('svg[role="img"]').scrollIntoViewIfNeeded(); await page.screenshot({ path: resolve(output, 'desktop-trend.png') });
    }
  }
  checks.push('all eight shared-filter report views, repository aggregation, source commit link, branch membership, heatmap and per-author trend');

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
    await page.getByLabel('洞察名称',{exact:false}).waitFor();
    const formWidth=await page.locator('form').evaluate(node=>node.getBoundingClientRect().width);
    assert(formWidth<=viewport.width+1,'shared edit form fits viewport');
    await page.goto(insightPath+'?analysis_report=heatmap');
    await report.locator('svg[role="img"]').waitFor();
  }
  checks.push('1280x800 and 390x844 viewport, document overflow, shared edit form bounds and screenshots');

  const insightCatalog = JSON.parse(await readFile(new URL('../src/shared/i18n/locales/insight-domain.json', import.meta.url), 'utf8'));
  const localeOrder = ['en-US', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY'];
  const insightLabel = (key, locale) => locale === 'zh-CN' ? key : insightCatalog[key][localeOrder.indexOf(locale)];
  const nativeNames = { 'en-US':'English','zh-CN':'简体中文','zh-TW':'繁體中文','ja-JP':'日本語','ko-KR':'한국어','ms-MY':'Bahasa Melayu' };
  const catalogs = Object.fromEntries(await Promise.all(Object.keys(nativeNames).map(async locale => [locale, JSON.parse(await readFile(new URL(`../src/shared/i18n/locales/${locale}.json`,import.meta.url),'utf8'))])));
  async function switchLocale(locale) {
    const active = new URL(page.url()).pathname.split('/')[1];
    await page.getByRole('button',{name:catalogs[active].language.label,exact:true}).click();
    await page.getByRole('menuitemradio',{name:nativeNames[locale],exact:true}).click();
    await page.waitForURL(url=>url.pathname.startsWith(`/${locale}/`));
  }
  await page.setViewportSize({ width: 1366, height: 900 });
  await page.goto(`${origin}/zh-CN/workspaces/2/code-insights/31?analysis_report=period`);
  for (const locale of ['en-US', 'zh-CN', 'zh-TW', 'ja-JP', 'ko-KR', 'ms-MY']) {
    await switchLocale(locale);
    const localizedReport = page.getByRole('region', { name: insightLabel('贡献分析报告', locale), exact: true });
    await localizedReport.getByRole('button', { name: insightLabel('周期统计', locale), exact: true }).waitFor();
    await localizedReport.locator('svg[role="img"]').waitFor();
    assert.equal(await localizedReport.locator('[data-author="Alice"]').count(), 1);
    assert.equal(await localizedReport.getByRole('group', { name: insightLabel('日期快捷条件（UTC）', locale), exact: true }).count(), 1);
    assert.equal(await localizedReport.getByRole('group', { name: insightLabel('日期快捷条件（UTC）', locale), exact: true }).getByRole('button', { name: insightLabel('全部', locale), exact: true }).count(), 1);
  }
  await switchLocale('zh-CN');
  checks.push('all six locales translate live report dimensions, date presets and chart labels while preserving author identities');

  source.job.effective_state = 'queued';
  await page.goto(insightPath);
  const taskStatus = page.getByRole('region', { name: '分析任务状态', exact: true });
  await taskStatus.getByRole('status').filter({hasText:'已排队'}).waitFor();
  assert.equal(await taskStatus.locator('.animate-spin').count(),0,'Queued work must not pretend to be running');
  const queuedJob=source.job;
  source.job=undefined;
  source.state='unavailable';
  source.last_error='Fixture: repository discovery awaits access restoration';
  await page.goto(insightPath);
  await taskStatus.getByRole('status').filter({hasText:'分析尚未完成'}).waitFor();
  assert.equal(await taskStatus.locator('details').getAttribute('open'),null,'Repository diagnostics remain collapsed by default');
  await taskStatus.locator('summary').click();
  await taskStatus.getByText('等待发现或恢复访问',{exact:true}).waitFor();
  assert.equal(await taskStatus.locator('details').getAttribute('open'),'','Repository diagnostics expand on demand');
  assert.equal((await taskStatus.innerText()).includes('unavailable'),false,'Unavailable is localized rather than shown as an unknown raw state');
  await taskStatus.getByText(source.last_error,{exact:true}).waitFor();
  source.job=queuedJob;
  source.state='partial';
  source.last_error='Fixture: one branch awaits complete diff';
  source.pipeline_required = true;
  source.job.effective_state = 'blocked';
  source.job.updated_at = now;
  await page.goto(insightPath);
  await taskStatus.locator('summary').click();
  await taskStatus.getByText('采集暂不可用', { exact: true }).waitFor();
  assert((await taskStatus.getByRole('status').innerText()).includes('分析尚未完成'));
  assert.equal((await taskStatus.innerText()).includes('流水线'),false,'Legacy blocked state must not require a business pipeline');
  for(const width of [1366,390]) {
    await page.setViewportSize({width,height:900});
    await taskStatus.evaluate(element=>element.scrollIntoView({block:'start'}));
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`Blocked status fits ${width}px`);
    await page.screenshot({path:resolve(output,`status-blocked-${width}.png`)});
  }
  await page.setViewportSize({width:1366,height:900});
  await taskStatus.getByRole('button', { name: '配置与同步', exact: true }).click();
  await page.getByRole('tab', { name: '配置与同步', exact: true }).waitFor();
  assert.equal(await page.getByRole('tab', { name: '配置与同步', exact: true }).getAttribute('aria-selected'), 'true');

  source.pipeline_required = false;
  source.job.effective_state = 'retry_wait';
  source.job.retry_at = now + 60000;
  await page.goto(insightPath);
  await taskStatus.locator('summary').click();
  await taskStatus.getByText('等待重试', { exact: true }).waitFor();
  await taskStatus.getByText('下次重试：', { exact: false }).waitFor();
  assert((await taskStatus.getByRole('status').innerText()).includes('等待重试'));
  assert.equal(await taskStatus.locator('.animate-spin').count(),0,'Retry backoff must not pretend to be running');

  source.job.effective_state = 'running';
  source.job.collector_phase = 'git_fetch';
  await page.goto(insightPath);
  await taskStatus.locator('summary').click();
  await taskStatus.getByText('同步中', { exact: true }).waitFor();
  await taskStatus.getByText('获取仓库变更', { exact: true }).waitFor();
  assert.equal(await taskStatus.getByRole('status').locator('.animate-spin').count(),1,'Running work has a progress indicator');
  const reportsBeforePublish = requests.filter(request => request.path.includes('/reports/')).length;
  source.state = 'complete'; source.last_error = ''; source.job.effective_state = 'complete';
  source.job.completed_generation = 2; source.synced_at = now + 1000;
  await taskStatus.getByRole('status').filter({ hasText: '同步完成' }).waitFor({ timeout: 22000 });
  assert.equal(await taskStatus.getByText('获取仓库变更',{exact:true}).count(),0,'Completed jobs hide stale collection phases');
  await page.waitForLoadState('networkidle');
  assert(requests.filter(request => request.path.includes('/reports/')).length >= reportsBeforePublish + 13, 'Published collection refreshes overview totals and all six branch/author pairs');
  assert.equal((await taskStatus.innerText()).includes('1970'), false);
  config.enabled = false;
  await page.goto(insightPath);
  await taskStatus.getByRole('status').filter({ hasText: '分析已暂停' }).waitFor();
  config.enabled = true;
  checks.push('queued/backoff states do not spin; unavailable/legacy blocked diagnostics never require business pipelines; actual running Git phase hides after completion; retry time and paused state; fifteen-second polling refreshes reports after publication');

  allowPipeline = false;
  const pipelineReadsBefore = requests.filter(request => request.path.endsWith('/pipelines')).length;
  await page.goto(insightPath+'/edit');
  await page.getByLabel('洞察名称',{exact:false}).waitFor();
  assert.equal(await page.getByLabel('分析流水线 team/repository',{exact:true}).count(),0);
  assert.equal(requests.filter(request=>request.path.endsWith('/pipelines')).length,pipelineReadsBefore);
  checks.push('limited role keeps saved memberships readable and never fetches pipeline options');
  allowRepositoryRead=false;
  await page.goto(insightPath);
  await page.getByRole('region',{name:'分析任务状态',exact:true}).waitFor();
  assert.equal(await page.getByRole('link',{name:'仓库事件',exact:true}).count(),0,'Repository events is hidden without repositories.read');
  checks.push('repository events navigation is above insights, reads all workspace events, and honors repository read permission');

  assert.deepEqual(unexpected, []); assert.deepEqual(errors, []); assert.deepEqual(consoleErrors, []);
  await writeFile(resolve(output, 'result.json'), JSON.stringify({ scope: 'Real Chromium application, fixture APIs only; no private provider or backend persistence claim', checks, pageErrors: errors, consoleErrors, screenshots: ['overview-top-1366.png','overview-cards-1366.png','overview-top-390.png','overview-cards-390.png','status-blocked-1366.png','status-blocked-390.png','desktop-trend.png', '1280-heatmap.png', '390-heatmap.png'] }, null, 2));
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
