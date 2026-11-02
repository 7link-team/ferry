/** Browser acceptance for insight creation, memberships and author mapping. Run npm run build first. */
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { preview } from "vite";
import { chromium } from "playwright";

const output = "data/maintenance/code-insights-ui";
await mkdir(output, { recursive: true });
const server = await preview({ preview: { host: "127.0.0.1", port: 0 } });
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
  "pipelines.read",
  "pipelines.run",
];
const errors = [];
const writes = [];
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
    if (repositoryMode === 'slow') {
      await new Promise(resolve => { releaseRepository = resolve; });
      await route.abort().catch(() => {});
      return;
    }
    if (repositoryMode === 'error') {
      await route.fulfill({ status: 502, json: { error: 'Provider unavailable' } });
      return;
    }
    if (repositoryMode === 'empty') {
      await route.fulfill({ json: { items: [], has_more: false } });
      return;
    }
    body = {
      items: [
        {
          name: "project",
          full_name: "team/project",
          url: "https://git.example.invalid/team/project.git",
          default_branch: "main",
        },
      ],
      has_more: false,
    };
  }
  if (path === "/workspaces/2/pipelines")
    body = { items: [{ id: "21", name: "Git analysis" }], total: 1 };
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
  await connection.selectOption('11');
  await page.locator('[role="status"]').filter({ hasText: '正在读取可见仓库' }).waitFor();
  await connection.selectOption('12');
  await page.getByLabel('Git 仓库地址', { exact: true }).fill('https://git.example.invalid/team/pending.git');
  await page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent.includes('加入洞察') && !button.disabled));
  releaseRepository();
  repositoryMode = 'empty';
  await connection.selectOption('11');
  await page.getByText('此连接没有可访问的仓库。请检查授权范围，或更换连接后重试。', { exact: true }).waitFor();
  repositoryMode = 'error';
  await page.getByRole('button', { name: '刷新连接', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'Provider unavailable' }).waitFor();
  repositoryMode = 'normal';
  await page.getByRole('button', { name: '重试读取仓库', exact: true }).click();
  await page.getByLabel("仓库连接", { exact: true }).selectOption("11");
  await page
    .getByLabel("添加洞察仓库", { exact: true })
    .selectOption("https://git.example.invalid/team/project.git");
  assert.equal(
    await submit.isDisabled(),
    true,
    "Choosing without adding must not submit an empty scope",
  );
  await page.getByRole("button", { name: "加入洞察", exact: true }).click();
  await page.getByText("已加入 1 个仓库", { exact: true }).waitFor();
  await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.disabled === false);
  assert.equal(
    await submit.isEnabled(),
    true,
    "Platform repository does not require a pipeline",
  );
  await page.getByLabel("仓库连接", { exact: true }).selectOption("12");
  await page
    .getByLabel("Git 仓库地址", { exact: true })
    .fill("https://git.example.invalid/team/other.git");
  await page.getByRole("button", { name: "加入洞察", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.disabled === true);
  assert.equal(
    await submit.isDisabled(),
    true,
    "Plain Git requires a pipeline when enabled",
  );
  await page
    .getByLabel("分析流水线 https://git.example.invalid/team/other.git", {
      exact: true,
    })
    .selectOption("21");
  await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.disabled === false);
  assert.equal(await submit.isEnabled(), true);
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
  await page.locator('select[name="update_mode"]').selectOption("event");
  assert.equal(
    await page.locator('input[aria-label="Cron 表达式"]').count(),
    0,
  );
  await page.locator('input[name="reconcile_minutes"]').waitFor();
  await submit.click();
  await page.waitForURL("**/code-insights");
  assert.equal(writes[0].data.repositories.length, 2);
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
  await page.getByRole("button", { name: "添加作者映射", exact: true }).click();
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
  permissions.splice(permissions.indexOf('code-insights.update'), 1);
  await page.reload();
  await page.getByText('Alice combined', { exact: true }).first().waitFor();
  assert.equal(await page.locator('input').count(), 0, 'Read-only members cannot edit mappings');
  assert.equal(await page.getByRole('button', { name: '保存', exact: true }).count(), 0);
  assert.deepEqual(errors, []);
  console.log(
    "PASS: insight creation, repository validation, responsive layout, event schedule, author mapping and contributor ID preservation",
  );
} finally {
  await browser.close();
  await server.httpServer.close();
}
