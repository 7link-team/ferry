/** Real Chromium acceptance for report forms and immutable result pages. */
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { createServer, preview } from "vite";
import { chromium } from "playwright";

const output = "data/maintenance/reports-reviews-ui";
await mkdir(output, { recursive: true });
const source = process.env.REPORTS_REVIEWS_UI_SOURCE === "1";
const server = source
  ? await createServer({ server: { host: "127.0.0.1", port: 0, open: false } })
  : await preview({ preview: { host: "127.0.0.1", port: 0, proxy: {} } });
if (source) await server.listen();
const origin = (
  process.env.REPORTS_REVIEWS_UI_URL || server.resolvedUrls.local[0]
).replace(/\/$/, "");
const chrome =
  process.env.PLAYWRIGHT_CHROME_PATH ||
  "C:/Program Files/Google/Chrome/Application/chrome.exe";
const browser = await chromium.launch({
  headless: true,
  ...(existsSync(chrome) ? { executablePath: chrome } : {}),
});
const page = await browser.newPage({ viewport: { width: 1366, height: 900 } });
const errors = [];
const writes = [];
page.on("pageerror", (e) => errors.push(e.message));
const permissions = [
  ...["insight-reports"].flatMap((resource) =>
    ["read", "create", "update", "delete", "generate", "run"].map(
      (action) => `${resource}.${action}`,
    ),
  ),
  "code-insights.read",
  "code-insights.update",
  "repositories.read",
  "credentials.read",
  "hosts.read",
  "host-groups.read",
  "pipelines.read",
];
const repositories = [
  { connection_id: "11", repository: "team/api" },
  { connection_id: "11", repository: "team/web" },
];
let report = {
  id: "r",
  name: "Engineering",
  insight_id: "31",
  dimension: "person",
  periods: ["week"],
  author_keys: ["alice"],
  applications: [],
  timezone: "Asia/Kuala_Lumpur",
  enabled: true,
  version: 1,
};
const now = Date.now();
const reviewEvidence = {
  available: true,
  total: 1,
  tests_passed: 0,
  tests_failed: 1,
  findings: 1,
  runs: [
    {
      id: "cr",
      review_id: "c",
      pipeline_run_id: "pr",
      repository: "team/api",
      head_sha: "bbb222",
      state: "completed",
      test_state: "failed",
      ai_state: "completed_findings",
      finding_count: 1,
    },
  ],
  unavailable_repositories: [],
};
const run = {
  id: "rr",
  definition_id: "r",
  period: "week",
  period_start: now - 7 * 86400000,
  period_end: now,
  created_at: now,
  state: "complete",
  definition_version: 1,
  generated_at: now,
  snapshot: {
    review_evidence: reviewEvidence,
    summary: {
      commits: 12,
      additions: 200,
      deletions: 30,
      effectives: 170,
      active_days: 4,
      repository_count: 2,
      contributor_count: 1,
    },
    previous: { commits: 8 },
    trend: [
      {
        date: "2026-11-01",
        commits: 5,
        additions: 80,
        deletions: 10,
        effectives: 70,
        authors: [
          {
            authorKey: "alice",
            nickname: "Alice Engineering",
            commits: 5,
            effectives: 70,
          },
        ],
      },
      {
        date: "2026-11-02",
        commits: 7,
        additions: 120,
        deletions: 20,
        effectives: 100,
        authors: [
          {
            authorKey: "alice",
            nickname: "Alice Engineering",
            commits: 7,
            effectives: 100,
          },
        ],
      },
    ],
    groups: [
      {
        key: "app",
        name: "Customer portal",
        commits: 12,
        active_days: 4,
        review_evidence: reviewEvidence,
      },
    ],
    commits: [
      {
        repository_id: "team/api",
        sha: "abc123",
        nickname: "Alice Engineering",
        author_date: now,
        message: "Add login tests",
      },
    ],
    coverage: {
      complete: false,
      sources: [{ repository: "team/api", state: "ready", synced_at: now }],
      missing_repositories: ["team/web"],
    },
    identity_version: 2,
    repository_scope_version: 1,
    timezone: "Asia/Kuala_Lumpur",
  },
};
const runDay = {
  ...run,
  id: "rd",
  period: "day",
  definition_version: 2,
  period_start: Date.UTC(2026, 10, 3),
  snapshot: {
    ...run.snapshot,
    summary: { commits: 3 },
    trend: [],
    commits: [
      {
        ...run.snapshot.commits[0],
        message: "Daily filter immutable evidence",
      },
    ],
  },
};
function envelope(items) {
  return { items, total: items.length, page: 1, page_size: 20 };
}
await page.route("**/api/**", async (route) => {
  const req = route.request();
  const path = new URL(req.url()).pathname.slice(4);
  const method = req.method();
  let body = envelope([]);
  if (method !== "GET") {
    const value = req.postDataJSON();
    writes.push({ path, method, value });
    if (
      path === "/workspaces/2/insight-reports" ||
      path === "/workspaces/2/insight-reports/r"
    ) {
      report = { ...report, ...value };
      body = report;
    }
  } else if (path === "/me")
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
  else if (path === "/setup") body = { required: false, has_users: true };
  else if (path.endsWith("/preferences"))
    body = { default_page_size: 20, dictionaries: {} };
  else if (path === "/workspaces/2/code-insights")
    body = envelope([{ id: "31", name: "Source insight" }]);
  else if (path === "/workspaces/2/insight-reports/options")
    body = {
      people: [
        {
          key: "alice",
          nickname: "Alice Engineering",
          name: "Alice Git",
          email: "alice@example.invalid",
          aliases: ["alice-old"],
        },
        { key: "bob", nickname: "Bob" },
      ],
      applications: [
        { id: "app", name: "Customer portal", repositories: [repositories[0]] },
      ],
      repositories,
    };
  else if (path === "/workspaces/2/insight-reports/r/runs/rr") body = run;
  else if (path === "/workspaces/2/insight-reports/r/runs/rd") body = runDay;
  else if (path === "/workspaces/2/insight-reports/r/runs") {
    const query = new URL(route.request().url()).searchParams;
    body = envelope(
      [run, runDay].filter(
        (row) =>
          (!query.get("period") || row.period === query.get("period")) &&
          (!query.get("period_start") ||
            new Date(row.period_start).toISOString().slice(0, 10) ===
              query.get("period_start")),
      ),
    );
  } else if (path === "/workspaces/2/insight-reports/r") body = report;
  else if (path === "/workspaces/2/insight-reports") body = envelope([report]);
  else if (path === "/workspaces/2/code-connections")
    body = envelope([
      {
        id: "11",
        name: "Team Git",
        provider: "git",
        url: "https://git.example.invalid/team/api.git",
      },
    ]);
  else if (path === "/workspaces/2/code-connections/11")
    body = {
      id: "11",
      name: "Team Git",
      provider: "git",
      url: "https://git.example.invalid/team/api.git",
    };
  else if (path === "/workspaces/2/credentials")
    body = envelope([{ id: "cred", name: "Audit token", kind: "token" }]);
  else if (path === "/workspaces/2/nodes")
    body = envelope([{ id: "host", name: "Build host" }]);
  else if (path === "/workspaces/2/node-groups")
    body = envelope([{ id: "group", name: "Build pool" }]);
  await route.fulfill({ json: body });
});
async function goto(path) {
  await page.goto(origin + "/en-US/workspaces/2/" + path);
}
async function native(label, value) {
  const control = page.getByLabel(label, { exact: true }).last();
  if ((await control.evaluate((element) => element.tagName)) === "SELECT") {
    await control.selectOption(value);
    assert.equal(await control.inputValue(), value);
  } else {
    const labelValue =
      {
        31: "Source insight",
        quarter: "Quarterly",
        day: "Daily",
        push: "Push events / detected changes",
        cred: "Audit token",
        host:
          label === "Runner placement" ? "Specific Runner host" : "Build host",
      }[value] ?? value;
    await control.click();
    await page.getByRole("option", { name: labelValue, exact: true }).click();
  }
}
async function noOverflow() {
  assert(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth + 1,
    ),
    "Page must not overflow viewport",
  );
}
try {
  await goto("insight-reports/new");
  await page.locator('input[name="name"]').fill("Delivery report");
  await native("Source insight", "31");
  await page.getByRole("checkbox", { name: /Alice Engineering/ }).check();
  await page.getByRole("checkbox", { name: "Daily", exact: true }).check();
  await page.getByRole("checkbox", { name: "Monthly", exact: true }).check();
  await page.getByRole("checkbox", { name: "Quarterly", exact: true }).check();
  await page.getByRole("checkbox", { name: "Yearly", exact: true }).check();
  await page
    .getByLabel("Search people and aliases", { exact: true })
    .fill("alice-old");
  assert.equal(
    await page.getByRole("checkbox", { name: "Bob", exact: true }).count(),
    0,
  );
  await page.getByRole("radio", { name: "Applications", exact: true }).check();
  await page
    .getByRole("checkbox", { name: "Customer portal", exact: true })
    .check();
  await page.getByRole("checkbox", { name: "team/web", exact: true }).check();
  await page.locator('input[name="timezone"]').fill("Asia/Kathmandu");
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await page.waitForURL(/insight-reports(\?|$)/);
  await goto("insight-reports/r");
  const reportCreate = writes.find(
    (w) => w.path.endsWith("/insight-reports") && w.method === "POST",
  );
  assert(reportCreate);
  assert.deepEqual(
    new Set(reportCreate.value.periods),
    new Set(["day", "week", "month", "quarter", "year"]),
  );
  assert.equal(reportCreate.value.dimension, "application");
  assert.equal(reportCreate.value.applications[0].repositories.length, 2);
  await goto("insight-reports/r/edit");
  await page.locator('input[name="name"]').fill("Updated delivery report");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.waitForURL(/insight-reports(\?|$)/);
  await goto("insight-reports/r");
  await page
    .getByRole("button", { name: "Generate report", exact: true })
    .click();
  await native("Period", "quarter");
  await page
    .getByLabel("Historical period start (optional)", { exact: true })
    .last()
    .fill("2026-07-01");
  await page.getByRole("button", { name: /Confirm.*Generate report/ }).click();
  assert(
    writes.some(
      (w) =>
        w.path.endsWith("/generate") &&
        w.value.period === "quarter" &&
        w.value.period_start === "2026-07-01",
    ),
  );
  await page.getByText("Incomplete data", { exact: false }).first().waitFor();
  await page.getByText("Add login tests", { exact: true }).waitFor();
  assert.equal(
    await page
      .getByRole("region", { name: "Test and review evidence", exact: true })
      .count(),
    0,
  );
  assert.equal(await page.locator("[data-list-toolbar]:visible").count(), 1);
  await native("Period", "day");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page
    .getByText("Daily filter immutable evidence", { exact: true })
    .waitFor();
  assert.equal(
    await page.getByText("Add login tests", { exact: true }).count(),
    0,
    "Filtering hides the old weekly snapshot",
  );
  await page
    .getByLabel("Historical period start (optional)", { exact: true })
    .fill("2026-11-03");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await page
    .getByText("Daily filter immutable evidence", { exact: true })
    .waitFor();
  await page.getByRole("tab", { name: "Run history", exact: true }).click();
  await page.locator("tbody button").first().click();
  await page
    .getByText("Daily filter immutable evidence", { exact: true })
    .waitFor();
  assert(
    new URL(page.url()).searchParams.get("run") === "rd",
    "History selects the immutable daily revision",
  );
  await page.getByRole("button", { name: "Clear", exact: true }).click();
  await page.getByRole("tab", { name: "Run history", exact: true }).click();
  await page.locator("tbody button").first().click();
  await page.getByText("Add login tests", { exact: true }).waitFor();
  const chartPoint = page.getByRole("button", {
    name: "2026-11-01: Commits 5",
    exact: true,
  });
  await chartPoint.scrollIntoViewIfNeeded();
  await chartPoint.hover();
  await page.getByRole("tooltip").waitFor();
  assert(
    (await page.getByRole("tooltip").innerText()).includes("Alice Engineering"),
  );
  await chartPoint.focus();
  assert(
    await chartPoint.evaluate((element) => document.activeElement === element),
    "Chart points are keyboard focusable",
  );
  assert((await page.getByRole("tooltip").innerText()).includes("80"));
  await page.keyboard.press("Escape");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Shift+Tab");
  await page.getByRole("tooltip").waitFor();
  assert(
    (await page.getByRole("tooltip").innerText()).includes("Alice Engineering"),
    "Keyboard focus opens contributor evidence",
  );
  await page.keyboard.press("Escape");
  await page.screenshot({
    path: output + "/report-desktop.png",
    fullPage: true,
  });
  for (const resource of ["insight-reports"]) {
    await goto(resource);
    const row = page.getByRole("row").nth(1);
    await row.getByRole("button", { name: "Delete", exact: true }).click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: /Confirm.*Delete/ })
      .click();
    assert(
      writes.some(
        (w) =>
          w.path.endsWith(
            resource === "insight-reports"
              ? "/insight-reports/r"
              : "/code-reviews/c",
          ) && w.method === "DELETE",
      ),
    );
  }
  await page.setViewportSize({ width: 375, height: 812 });
  for (const dark of [false, true]) {
    await goto("insight-reports/r/edit");
    await page.evaluate(
      (value) => document.documentElement.classList.toggle("dark", value),
      dark,
    );
    await noOverflow();
    assert.equal(
      await page
        .locator("[data-form-actions]")
        .evaluate((element) => getComputedStyle(element).position),
      "sticky",
    );
    await page.locator('input[name="name"]').focus();
    await page.screenshot({
      path: output + `/report-mobile-${dark ? "dark" : "light"}.png`,
      fullPage: true,
    });
  }
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 844, height: 390 });
  await goto("insight-reports/r");
  await noOverflow();
  assert.deepEqual(errors, []);
  await writeFile(
    output + "/result.json",
    JSON.stringify(
      { mode: "mocked_api_browser", passed: true, writes, errors },
      null,
      2,
    ),
  );
  console.log(
    "PASS report CRUD controls, scopes, snapshots, charts and mobile viewport",
  );
} catch (error) {
  console.error(errors);
  console.error((await page.locator("body").innerText()).slice(0, 8000));
  await page.screenshot({ path: output + "/failure.png", fullPage: true });
  throw error;
} finally {
  await browser.close();
  await (server.close
    ? server.close()
    : new Promise((resolve) => server.httpServer.close(resolve)));
}
