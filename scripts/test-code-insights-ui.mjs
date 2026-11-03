/** Browser acceptance for insight creation, memberships and author mapping. Run npm run build first. */

async function chooseNative(control, value) {
  await control.waitFor();
  await control.page().waitForFunction(({ element, expected }) => !element.disabled && [...element.options].some(option => option.value === expected), { element: await control.elementHandle(), expected: value });
  const index = await control.evaluate((element, expected) => [...element.options].findIndex(option => option.value === expected), value);
  assert(index >= 0, `Missing option ${value}`);
  await control.click();
  await control.press('Home');
  for (let i = 0; i < index; i++) await control.press('ArrowDown');
  await control.press('Enter');
  assert.equal(await control.inputValue(), value, 'Native selection must persist');
}

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { createServer, preview } from "vite";
import { chromium } from "playwright";

const output = "data/maintenance/code-insights-ui";
await mkdir(output, { recursive: true });
const sourceMode = process.env.CODE_INSIGHTS_UI_SOURCE === "1";
const server = sourceMode ? await createServer({ server: { host: "127.0.0.1", port: 0, open: false } }) : await preview({ preview: { host: "127.0.0.1", port: 0 } });
if (sourceMode) await server.listen();
const origin = server.resolvedUrls.local[0].replace(/\/$/, "");
const chrome =
  process.env.PLAYWRIGHT_CHROME_PATH ||
  "C:/Program Files/Google/Chrome/Application/chrome.exe";
const browser = await chromium.launch({
  headless: true,
  ...(existsSync(chrome) ? { executablePath: chrome } : {}),
});
const page = await browser.newPage({ viewport: { width: 1366, height: 900 } });
const permissions = [
  "code-insights.read",
  "code-insights.create",
  "code-insights.update",
  "code-insights.sync",
  "repositories.read",
];
const errors = [];
const writes = [];
const repositoryRequests = [];
let repositoryMode = 'normal';
let releaseRepository;
page.on("pageerror", (error) => errors.push(error.message));
let insight = {
  id: "31",
  name: "Engineering",
  enabled: true,
  update_mode: "scheduled",
  schedule: "0 */4 * * *",
  timezone: "",
  full_history: false,
  initial_days: 90,
  reconcile_minutes: 1440,
  repositories: [
    {
      connection_id: "11",
      provider: "github",
      repository: "https://git.example.invalid/team/project.git",
      pipeline_id: "",
    },
  ],
  aliases: [
    {
      email: "alice@example.invalid",
      nickname: "Alice",
      contributor_id: "123",
    },
  ],
  version: 1,
  sources: [],
};
await page.route("**/api/**", async (route) => {
  const req = route.request();
  const path = new URL(req.url()).pathname.slice(4);
  let body = { items: [], total: 0, page: 1, page_size: 100 };
  if (path === "/me")
    body = {
      user: { id: "1", name: "Operator", is_superadmin: true },
      workspaces: [
        {
          id: "2",
          slug: "ops",
          name: "Operations",
          permissions,
          role_name: "Owner",
        },
      ],
    };
  if (path === "/setup") body = { required: false, has_users: true };
  if (path.endsWith("/preferences"))
    body = { default_page_size: 20, dictionaries: {} };
  if (path === "/workspaces/2/code-connections")
    body = {
      items: [
        { id: "11", name: "Codeup team", provider: "codeup", state: "connected" },
        { id: "12", name: "Git source", provider: "git" },
      ],
      total: 2,
      page_size: 200,
    };
  if (path === "/workspaces/2/code-connections/11/repositories") {
    const repositoryPage = Number(new URL(req.url()).searchParams.get('page') || 1);
    repositoryRequests.push(repositoryPage);
    if (repositoryMode === 'slow') {
      await new Promise(resolve => { releaseRepository = resolve; });
      await route.abort().catch(() => {});
      return;
    }
    if (repositoryMode === 'error') {
      await route.fulfill({ status: 502, json: { error: 'Provider unavailable' } });
      return;
    }
    if (repositoryMode === 'secondPageError' && repositoryPage === 2) {
      await route.fulfill({ status: 502, json: { error: 'Second page unavailable' } });
      return;
    }
    if (repositoryMode === 'empty') {
      await route.fulfill({ json: { items: [], has_more: false } });
      return;
    }
    const names = repositoryPage === 1 ? ['team/project', 'team/mobile/library'] : ['team/mobile/app', 'other/service'];
    body = {
      items: names.map(full_name => ({ name: full_name.split('/').at(-1), full_name, url: `https://git.example.invalid/${full_name}.git`, default_branch: 'main' })),
      has_more: repositoryPage === 1,
    };
  }
  if (path === "/workspaces/2/pipelines") assert.fail("Insight collection must not request business pipelines");
  if (path === "/workspaces/2/code-insights/31") body = insight;
  if (
    ["POST", "PATCH"].includes(req.method()) &&
    path.includes("/code-insights")
  ) {
    const data = req.postDataJSON();
    writes.push({ method: req.method(), path, data });
    insight = { ...insight, ...data, version: insight.version + 1 };
    body = { id: "31" };
  }
  await route.fulfill({ json: body });
});
try {
  await page.goto(origin + "/zh-CN/workspaces/2/code-insights/new");
  const submit = page.getByRole("button", { name: "创建", exact: true });
  await submit.waitFor();
  assert.equal(await submit.isDisabled(), true);
  assert.equal(await page.getByLabel("作者昵称映射").count(), 0);
  await page.locator('input[name="name"]').fill("Engineering");
  const connection = page.getByLabel("仓库连接", { exact: true });
  repositoryMode = 'slow';
  await connection.click();
  await page.keyboard.press('Home');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  assert.equal(await connection.inputValue(), '11', 'Real mouse/keyboard selection must retain the connection');
  await page.locator('[role="status"]').filter({ hasText: '正在读取可见仓库' }).waitFor();
  await chooseNative(connection, '12');
  await page.getByLabel('Git 仓库地址', { exact: true }).fill('https://git.example.invalid/team/pending.git');
  await page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent.includes('加入洞察') && !button.disabled));
  releaseRepository();
  repositoryMode = 'empty';
  await chooseNative(connection, '11');
  await page.getByText('此连接没有可访问的仓库。请检查授权范围，或更换连接后重试。', { exact: true }).waitFor();
  repositoryMode = 'error';
  await page.getByRole('button', { name: '刷新连接', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'Provider unavailable' }).waitFor();
  repositoryMode = 'secondPageError';
  const refreshConnection=page.getByRole('button',{name:'刷新连接',exact:true});
  assert.equal((await refreshConnection.textContent()).trim(),'','Refresh has an icon without an outside text button');
  assert.equal(await refreshConnection.evaluate(element=>!!element.closest('[data-slot="select-refresh"]')?.querySelector('select[aria-label="仓库连接"]')),true,'Refresh is embedded beside its corresponding select');
  await refreshConnection.click();
  await page.getByRole('alert').filter({ hasText: 'Second page unavailable' }).waitFor();
  assert.equal(await page.getByRole('button',{name:'选择仓库',exact:true}).isDisabled(),true,'A failed later page must not expose a partial repository list');
  assert.equal(await page.getByLabel('仓库分组', { exact: true }).isDisabled(), true);
  repositoryMode = 'normal';
  await refreshConnection.click();
  await chooseNative(page.getByLabel("仓库连接", { exact: true }), "11");
  const repositoryPicker=page.getByRole('button',{name:'选择仓库',exact:true});
  await page.waitForFunction(()=>!document.querySelector('button[aria-label="选择仓库"]')?.disabled);
  assert.equal(await connection.inputValue(),'11','Refresh retains the selected connection');
  assert.equal(writes.length,0,'Refreshing a connection cannot submit the form');
  assert.equal(await page.getByRole('button',{name:'加入洞察',exact:true}).count(),0,'Platform checkbox selection adds immediately without an extra add action');
  await repositoryPicker.click();
  await page.getByRole('menuitemcheckbox',{name:'team/project',exact:true}).click();
  await page.getByText("已加入 1 个仓库", { exact: true }).first().waitFor();
  await page.getByRole('menuitemcheckbox',{name:'team/mobile/library',exact:true}).click();
  await page.getByText('已加入 2 个仓库',{exact:true}).first().waitFor();
  await page.getByRole('menuitemcheckbox',{name:'team/mobile/library',exact:true}).click();
  await page.getByText('已加入 1 个仓库',{exact:true}).first().waitFor();
  await page.getByRole('textbox',{name:'搜索仓库',exact:true}).fill('mobile');
  assert.equal(await page.getByRole('menuitemcheckbox',{name:'team/project',exact:true}).count(),0);
  await page.getByRole('menuitemcheckbox',{name:'选择当前结果（2）',exact:true}).click();
  await page.getByText('已加入 3 个仓库',{exact:true}).first().waitFor();
  await page.keyboard.press('Escape');
  assert(repositoryRequests.includes(2), 'Repository pages are loaded automatically');
  assert.equal(await page.getByRole('button', { name: '加载更多仓库', exact: true }).count(), 0);
  const group = page.getByLabel('仓库分组', { exact: true });
  assert.deepEqual(await group.locator('option').evaluateAll(options => options.map(option => option.value)), ['', 'other', 'team', 'team/mobile']);
  await chooseNative(group, 'team/mobile');
  await repositoryPicker.click();
  assert.deepEqual(await page.getByRole('menuitemcheckbox').allTextContents(),['team/mobile 下全部仓库','选择当前结果（2）','team/mobile/library','team/mobile/app']);
  assert.equal(await page.getByRole('textbox',{name:'搜索仓库',exact:true}).inputValue(),'','Changing group resets the repository search');
  const selectGroup=page.getByRole('menuitemcheckbox',{name:'选择当前结果（2）',exact:true});
  assert.equal(await selectGroup.getAttribute('aria-checked'),'true');
  await selectGroup.click();
  await page.getByText('已加入 1 个仓库',{exact:true}).first().waitFor();
  await selectGroup.click();
  await page.getByText('已加入 3 个仓库', { exact: true }).first().waitFor();
  await page.keyboard.press('Escape');
  await chooseNative(group,'team');
  await repositoryPicker.click();
  await page.getByRole('menuitemcheckbox',{name:'team/project',exact:true}).click();
  await page.getByRole('menuitemcheckbox',{name:'选择当前结果（3）',exact:true}).click();
  await page.keyboard.press('Escape');
  await page.getByText('已加入 3 个仓库',{exact:true}).first().waitFor();
  await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.disabled === false);
  assert.equal(
    await submit.isEnabled(),
    true,
    "Platform repository does not require a pipeline",
  );
  await chooseNative(page.getByLabel("仓库连接", { exact: true }), "12");
  assert.equal(await page.getByLabel('仓库分组', { exact: true }).count(), 0, 'Plain Git has no provider namespace selector');
  await page
    .getByLabel("Git 仓库地址", { exact: true })
    .fill("https://git.example.invalid/team/other.git");
  await page.getByRole("button", { name: "加入洞察", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.disabled === false);
  assert.equal(
    await submit.isEnabled(),
    true,
    "Plain Git saves without a business pipeline or host",
  );
  await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.disabled === false);
  assert.equal(await submit.isEnabled(), true);
  await chooseNative(connection,'11');
  assert.equal(await group.inputValue(),'','Changing the connection resets its namespace filter');
  await page.waitForFunction(()=>!document.querySelector('button[aria-label="选择仓库"]')?.disabled);
  await repositoryPicker.click();
  await page.getByRole('menuitemcheckbox',{name:'other/service',exact:true}).click();
  await page.getByRole('menuitemcheckbox',{name:'other/service',exact:true}).click();
  await page.keyboard.press('Escape');
  await page.getByText('已加入 4 个仓库',{exact:true}).first().waitFor();
  assert.equal(await page.locator('[aria-label^="分析流水线"]').count(),0,'Repository selection has no business pipeline controls');
  assert.equal(writes.length,0,'Repository selections do not submit the insight');
  for (const width of [1366, 768, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth + 1,
      ),
      `No overflow at ${width}px`,
    );
    await page.evaluate(() =>
      document.querySelectorAll("*").forEach((element) => {
        if (element.scrollTop) element.scrollTop = 0;
      }),
    );
    await page.screenshot({ path: `${output}/create-${width}.png` });
  }
  await page.setViewportSize({ width: 1366, height: 900 });
  await page.getByRole('combobox', { name: /^更新方式/ }).click();
  await page.getByRole('option', { name: '事件更新', exact: true }).click();
  assert.equal(
    await page.locator('input[aria-label="Cron 表达式"]').count(),
    0,
  );
  await page.locator('input[name="reconcile_minutes"]').waitFor();
  await submit.click();
  await page.waitForURL("**/code-insights");
  assert.equal(writes[0].data.repositories.length, 4);
  assert.deepEqual(writes[0].data.repositories.map(row => row.repository).sort(), ['team/project', 'team/mobile/library', 'team/mobile/app', 'https://git.example.invalid/team/other.git'].sort());
  assert.equal(writes[0].data.repositories.find(row=>row.repository==='team/mobile/library').pipeline_id,'');
  assert.equal(writes[0].data.repositories.find(row=>row.connection_id==='12').pipeline_id,'');
  assert.deepEqual(writes[0].data.aliases, []);
  assert.equal(writes[0].data.update_mode, "event");

  insight.aliases = [
    {
      email: "alice@example.invalid",
      nickname: "Alice",
      contributor_id: "123",
    },
  ];
  await page.goto(origin + "/zh-CN/workspaces/2/code-insights/31?tab=authors");
  await page.getByLabel("统一昵称 1", { exact: true }).fill("Alice combined");
  assert.equal(await page.locator("textarea").count(), 0);
  const save = page.getByRole("button", { name: "保存", exact: true });
  await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/code-insights/31") && r.request().method() === "GET",
    ),
    save.click(),
  ]);
  assert(page.url().endsWith("?tab=authors"), "Save stays on mapping tab");
  assert.equal(writes.at(-1).data.aliases[0].contributor_id, "123");
  assert.equal(writes.at(-1).data.aliases[0].nickname, "Alice combined");
  await page.getByRole("button", { name: "在第 1 行下方添加映射", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelector('button[type="submit"]')?.disabled,
  );
  assert.equal(await save.isDisabled(), true);
  await page
    .getByLabel("提交邮箱 2", { exact: true })
    .fill("alice2@example.invalid");
  await page.getByLabel("统一昵称 2", { exact: true }).fill("Alice combined");
  assert.equal(await save.isEnabled(), true);
  for (const width of [1366, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    assert(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth + 1,
      ),
    );
    await page.screenshot({ path: `${output}/authors-${width}.png` });
  }
  const nextVersion = insight.version;
  await Promise.all([
    page.waitForResponse(r => r.url().endsWith('/code-insights/31') && r.request().method() === 'GET'),
    save.click(),
  ]);
  assert.equal(writes.at(-1).data.version, nextVersion, 'Second save uses refreshed optimistic-lock version');
  assert.equal(writes.at(-1).data.aliases.length, 2);
  await page.goto(origin + '/zh-CN/workspaces/2/code-insights/new');
  await page.locator('input[name="name"]').fill('Dynamic scope');
  await chooseNative(page.getByLabel('仓库连接', {exact:true}), '11');
  await chooseNative(page.getByLabel('仓库分组', {exact:true}), 'team/mobile');
  await page.getByRole('button', {name:'选择仓库',exact:true}).click();
  await page.getByRole('menuitemcheckbox', {name:'team/mobile 下全部仓库',exact:true}).click();
  for (const path of ['team/mobile/app','team/mobile/library']) {
    const item=page.getByRole('menuitemcheckbox',{name:path,exact:true});
    assert.equal(await item.getAttribute('aria-checked'),'true');
    assert.equal(await item.getAttribute('aria-disabled'),'true');
  }
  await page.keyboard.press('Escape');
  await Promise.all([
    page.waitForResponse(response=>response.url().endsWith('/code-insights')&&response.request().method()==='POST'),
    page.getByRole('button',{name:'创建',exact:true}).click(),
  ]);
  assert.deepEqual(writes.at(-1).data.repositories,[{connection_id:'11',repository:'team/mobile/*',all_repositories:false,pipeline_id:'',provider:'codeup'}]);

  const catalog=JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/insight-domain.json',import.meta.url),'utf8'));
  const locales=['en-US','zh-CN','zh-TW','ja-JP','ko-KR','ms-MY'];
  const order=['en-US','zh-TW','ja-JP','ko-KR','ms-MY'];
  const label=(key,locale)=>locale==='zh-CN'?key:catalog[key][order.indexOf(locale)];
  for(const locale of locales)for(const width of [1366,320]) {
    await page.setViewportSize({width,height:900});
    await page.goto(`${origin}/${locale}/workspaces/2/code-insights/new`);
    const mode=page.locator('[data-auto-field="update_mode"] [data-slot="select-trigger"]');
    await mode.waitFor();
    const before=await mode.boundingBox();
    await mode.click();
    await page.getByRole('option',{name:label('事件更新',locale),exact:true}).click();
    const after=await mode.boundingBox();
    assert(Math.abs(before.width-after.width)<1,`${locale}/${width}: intrinsic width covers both options without jumping`);
    const sizing=await mode.evaluate(element=>{
      const style=getComputedStyle(element),canvas=document.createElement('canvas'),context=canvas.getContext('2d');context.font=style.font;
      // Include the actual placeholder/unknown-value text reserved by AutoForm,
      // which can be longer than either option in languages such as Japanese.
      const labels=element.querySelector('[data-sizer]').getAttribute('data-sizer').split('\n');
      const longest=Math.max(...labels.map(text=>context.measureText(text).width));
      const value=element.querySelector('[data-slot="select-value"]');
      return{width:element.getBoundingClientRect().width,parent:element.closest('[data-auto-fields]').getBoundingClientRect().width,longest,chrome:parseFloat(style.paddingLeft)+parseFloat(style.paddingRight)+16+parseFloat(style.columnGap)+2,clipped:value.scrollWidth>value.clientWidth+1};
    });
    assert(sizing.width<=Math.min(sizing.parent,Math.max(128,sizing.longest+sizing.chrome))+4,`${locale}/${width}: a short choice must not fill an entire form column: ${JSON.stringify(sizing)}`);
    assert.equal(sizing.clipped,false,`${locale}/${width}: selected label remains readable`);
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`${locale}/${width}: form has no horizontal overflow`);
    if(locale==='zh-CN')await page.screenshot({path:`${output}/strategy-compact-${width}.png`});
  }
  insight.aliases = [{ email: "alice@example.invalid", nickname: "Alice combined", contributor_id: "123" }];
  await page.goto(origin + '/zh-CN/workspaces/2/code-insights/31?tab=authors');
  permissions.splice(permissions.indexOf('code-insights.update'), 1);
  await page.reload();
  await page.getByText('Alice combined', { exact: true }).first().waitFor();
  assert.equal(await page.locator('input').count(), 0, 'Read-only members cannot edit mappings');
  assert.equal(await page.getByRole('button', { name: '保存', exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  console.log(
    "PASS: insight creation without business pipelines or hosts, automatic pagination, multi-select immediate add/uncheck/search/group select-all/dedup, other connection retention, embedded refresh, later-page failure/retry, repository validation, six-locale intrinsic select width and320px layout, event schedule, author mapping and contributor ID preservation",
  );
} finally {
  await browser.close();
  if (sourceMode) await server.close();
  else await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()));
}
