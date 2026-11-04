/** Live product API browser acceptance. Provider fixture is owned by the Python launcher. */
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, writeFile, readFile } from "node:fs/promises";
import { chromium } from "playwright";
const origin = process.env.LIVE_BASE_URL,
  workspace = process.env.LIVE_WORKSPACE,
  insight = process.env.LIVE_INSIGHT_ID;
assert(
  origin && workspace && insight,
  "Run through test-reports-reviews-live.py",
);
const output =
  process.env.LIVE_OUTPUT || "data/maintenance/reports-reviews-live";
await mkdir(output, { recursive: true });
const chrome =
  process.env.PLAYWRIGHT_CHROME_PATH ||
  "C:/Program Files/Google/Chrome/Application/chrome.exe";
const browser = await chromium.launch({
  headless: true,
  ...(existsSync(chrome) ? { executablePath: chrome } : {}),
});
const context = await browser.newContext({
  viewport: { width: 1366, height: 900 },
});
const page = await context.newPage();
const errors = [],
  writes = [];
page.on("pageerror", (e) => errors.push(e.message));
page.on("response", (response) => {
  if (response.request().method() !== "GET" && response.url().includes("/api/"))
    writes.push({
      url: new URL(response.url()).pathname,
      method: response.request().method(),
      status: response.status(),
    });
});
const apiBase = `${origin}/api/workspaces/${workspace}`;
const headers = { "X-Ferry-Request": "1" };
const get = async (path) => {
  const response = await context.request.get(apiBase + path);
  assert(
    response.ok(),
    `${path}: ${response.status()} ${await response.text()}`,
  );
  return response.json();
};
const goto = async (path) => {
  await page.goto(`${origin}/en-US/workspaces/${workspace}/${path}`);
};
async function select(label, value) {
  await page.getByLabel(label, { exact: true }).last().click();
  await page.getByRole("option", { name: value, exact: true }).click();
}
try {
  const login = await context.request.post(origin + "/api/sessions", {
    headers,
    data: {
      email: process.env.LIVE_EMAIL,
      password: process.env.LIVE_PASSWORD,
    },
  });
  assert(login.ok(), await login.text());
  await goto("insight-reports/new");
  const source = page.locator('select[name="insight_id"]');
  await source
    .locator(`option[value="${insight}"]`)
    .waitFor({ state: "attached" });
  await source.selectOption(insight);
  await page.locator('input[name="name"]').fill("Live identity report");
  assert(
    await page
      .getByRole("button", { name: "Create", exact: true })
      .isDisabled(),
    "Empty subject scope must block create",
  );
  await page.getByRole("checkbox", { name: /Combined/ }).check();
  await page.getByRole("checkbox", { name: "Yearly", exact: true }).check();
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await page.waitForURL(/insight-reports(\?|$)/);
  const definition = (await get("/insight-reports")).items.find(
    (row) => row.name === "Live identity report",
  );
  assert(definition);
  assert.deepEqual(
    definition.applications,
    [],
    "changing the source must reset applications to an array",
  );
  const report = definition.id;
  await goto(`insight-reports/${report}/edit`);
  await page.locator('input[name="name"]').fill("Live identity report edited");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await page.waitForURL(/insight-reports(\?|$)/);
  await goto(`insight-reports/${report}`);
  await page
    .getByRole("button", { name: "Generate report", exact: true })
    .click();
  await select("Period", "Yearly");
  await page
    .getByLabel("Historical period start (optional)", { exact: true })
    .last()
    .fill("2025-01-01");
  await page
    .getByRole("button", { name: "Confirm Generate report", exact: true })
    .click();
  await page
    .getByText("Combined", { exact: true })
    .first()
    .waitFor({ timeout: 60000 });
  await page.reload();
  await page.getByText("Combined", { exact: true }).first().waitFor();
  const runs = await get(`/insight-reports/${report}/runs`);
  assert.equal(runs.items[0].state, "complete");
  const generated = await get(
    `/insight-reports/${report}/runs/${runs.items[0].id}`,
  );
  assert.equal(generated.snapshot.summary.commits, 2);
  assert.equal(generated.snapshot.groups[0].name, "Combined");
  assert.equal(generated.snapshot.timezone, definition.timezone);
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page
      .getByRole("link", { name: "Export report · CSV", exact: true })
      .click(),
  ]);
  await download.saveAs(output + "/report.csv");
  assert((await readFile(output + "/report.csv", "utf8")).includes("Combined"));
  await page.screenshot({ path: output + "/live-report.png", fullPage: true });
  await goto("insight-reports/new");
  await page.locator('select[name="insight_id"]').selectOption(insight);
  await page.locator('input[name="name"]').fill("Live application report");
  await page.getByRole("radio", { name: "Applications", exact: true }).check();
  await page
    .getByRole("checkbox", { name: "Fixture application", exact: true })
    .check();
  await page
    .getByRole("checkbox", { name: "owner/repository", exact: true })
    .check();
  await page.getByRole("button", { name: "Create", exact: true }).click();
  await page.waitForURL(/insight-reports(\?|$)/);
  const application = (await get("/insight-reports")).items.find(
    (row) => row.name === "Live application report",
  );
  assert(application);
  assert.deepEqual(application.author_keys, []);
  assert.equal(
    application.applications[0].repositories[0].repository,
    "owner/repository",
  );
  assert.equal(
    (await context.request.get(apiBase + "/code-reviews")).status(),
    404,
  );
  for (const path of [
    "/code-reviews",
    "/code-reviews/1",
    "/code-reviews/1/run",
    "/code-reviews/1/runs/1/cancel",
  ]) {
    const response = await context.request.post(apiBase + path, {
      headers,
      data: {},
    });
    assert.equal(
      response.status(),
      404,
      `Retired endpoint must be absent: ${path}`,
    );
  }
  assert.equal(await page.locator('a[href*="/code-reviews"]').count(), 0);
  for (const [resource, id] of [
    ["insight-reports", report],
    ["insight-reports", application.id],
  ]) {
    await goto(resource);
    await page
      .getByRole("row")
      .filter({
        hasText:
          id === report
            ? "Live identity report edited"
            : "Live application report",
      })
      .getByRole("button", { name: "Delete", exact: true })
      .click();
    await Promise.all([
      page.waitForResponse(
        (response) =>
          response.request().method() === "DELETE" &&
          response.url().endsWith(`/${resource}/${id}`),
      ),
      page
        .getByRole("dialog")
        .getByRole("button", { name: /Confirm.*Delete/ })
        .click(),
    ]);
    assert(!(await get(`/${resource}`)).items.some((row) => row.id === id));
  }
  assert.deepEqual(errors, []);
  await writeFile(
    output + "/result.json",
    JSON.stringify(
      {
        mode: "real_product_api_browser",
        provider: "local HTTP provider fixture",
        passed: true,
        generated,
        writes,
        errors,
      },
      null,
      2,
    ),
  );
  console.log(
    "PASS real backend Chromium report CRUD, ordinary source selection, generated alias snapshot, refresh and retired reviews",
  );
} catch (error) {
  await writeFile(
    output + "/result.json",
    JSON.stringify(
      {
        mode: "real_product_api_browser",
        passed: false,
        error: String(error),
        writes,
        errors,
      },
      null,
      2,
    ),
  );
  await page.screenshot({ path: output + "/failure.png", fullPage: true });
  console.error((await page.locator("body").innerText()).slice(0, 10000));
  throw error;
} finally {
  await browser.close();
}
