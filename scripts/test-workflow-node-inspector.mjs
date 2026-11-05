import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { mkdtemp, open, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import assert from "node:assert/strict";

const directory = await mkdtemp(join(tmpdir(), "workflow-inspector-"));
const listener = createServer();
await new Promise((r) => listener.listen(0, "127.0.0.1", r));
const port = listener.address().port;
await new Promise((r) => listener.close(r));
const external = process.env.FERRY_TEST_ORIGIN,
  origin = external || `http://127.0.0.1:${port}`;
const log = await open(join(directory, "server.log"), "w");
const env = {
  ...process.env,
  FERRY_DATA_DIR: directory,
  FERRY_BIND: `127.0.0.1:${port}`,
  FERRY_SANDBOX_PATH: resolve(
    `target/debug/ferry-sandbox${process.platform === "win32" ? ".exe" : ""}`,
  ),
};
for (const key of [
  "FERRY_DATABASE_URL",
  "FERRY_NATS_URL",
  "FERRY_CLUSTER_NAMESPACE",
  "FERRY_ARTIFACT_STORE",
])
  delete env[key];
const server = external
  ? undefined
  : spawn(
      resolve(
        `target/debug/ferry-server${process.platform === "win32" ? ".exe" : ""}`,
      ),
      [],
      { env, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
    );
const messages = JSON.parse(
  await readFile("src/shared/i18n/locales/workflow-domain.json", "utf8"),
);
const t = (key) => messages[key][1];
const checks = [],
  pass = (name) => {
    checks.push(name);
    console.log("PASS", name);
  };
let browser;
const node = (id, type, params, x) => ({
  id,
  name: id,
  type,
  type_version: 1,
  params,
  position: [x, 160],
});
const link = (from, to, index = 0, port = "main") => ({
  from: { node: from, port: "main", index: 0 },
  to: { node: to, port, index },
});
try {
  for (let i = 0; ; i++) {
    if (server) assert.equal(server.exitCode, null, "server exited");
    try {
      if ((await fetch(origin + "/api/status")).ok) break;
    } catch {}
    assert(i < 200, "startup timeout");
    await new Promise((r) => setTimeout(r, 100));
  }
  browser = await chromium.launch({
    channel: process.env.FERRY_BROWSER_CHANNEL || "chrome",
    headless: true,
  });
  const context = await browser.newContext({
    viewport: { width: 1600, height: 1000 },
  });
  const headers = { "X-Ferry-Request": "1", "Accept-Language": "zh-CN" };
  const api = async (
    path,
    data,
    method = data ? "POST" : "GET",
    expected = 200,
  ) => {
    const response = await context.request.fetch(origin + "/api" + path, {
      method,
      headers,
      timeout: 10000,
      ...(data ? { data } : {}),
    });
    assert.equal(
      response.status(),
      expected,
      path + ": " + (await response.text()),
    );
    return expected === 204 ? undefined : response.json();
  };
  if (external) {
    const access = JSON.parse(
      await readFile(process.env.FERRY_TEST_ACCESS_FILE, "utf8"),
    );
    await api("/sessions", { email: access.email, password: access.password });
  } else
    await api("/setup", {
      name: "Inspector acceptance",
      email: "inspector@example.invalid",
      password: "temporary-inspector-password",
      token: (await readFile(join(directory, "setup-token"), "utf8")).trim(),
    });
  const workspace = await api("/workspaces", {
    name: "Node inspector acceptance",
    slug: "inspector-" + Date.now(),
  });
  const root = `/workspaces/${workspace.id}`;
  const graph = {
    engine: "ferry-graph",
    schema: 1,
    nodes: [
      node(
        "start",
        "core.trigger.manual",
        {
          input_schema: {
            type: "object",
            properties: {
              profile: {
                type: "object",
                properties: { name: { type: "string" } },
              },
            },
          },
        },
        0,
      ),
      node(
        "assign",
        "data.set",
        {
          keep_other_fields: true,
          fields: [{ path: "greeting", value: "={{ $json.profile.name }}" }],
        },
        320,
      ),
      node("finish", "core.no_op", {}, 640),
    ],
    connections: [link("start", "assign"), link("assign", "finish")],
  };
  let row = await api(
    `${root}/workflows`,
    { name: "Input · parameters · output", definition: JSON.stringify(graph) },
    "POST",
    201,
  );
  const endpoint = `${root}/workflows/${row.id}`;
  const run = async (current, inputs = { profile: { name: "Ada" } }) => {
    const started = await api(
      `${root}/workflows/${current.id}/executions`,
      {
        mode: "manual",
        revision: "draft",
        version: current.version,
        entry_node_id: "start",
        inputs,
      },
      "POST",
      201,
    );
    let result;
    for (let i = 0; i < 400; i++) {
      result = await api(`${root}/workflow-executions/${started.id}`);
      if (["succeeded", "failed", "cancelled"].includes(result.state)) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    assert.equal(result.state, "succeeded", JSON.stringify(result));
    return result;
  };
  const execution = await run(row);
  const attemptList = await api(
    `${root}/workflow-executions/${execution.id}/node-runs`,
  );
  const attempt = attemptList.items.find((item) => item.node_id === "assign");
  const detailPath = `${root}/workflow-executions/${execution.id}/node-runs/${attempt.id}/data`;
  const data = await api(detailPath);
  assert.deepEqual(data.input.items[0].json, { profile: { name: "Ada" } });
  assert.equal(data.resolved_params[0].fields[0].value, "Ada");
  assert.equal(data.output.ports[0].items[0].json.greeting, "Ada");
  assert.match(data.input.sources[0].iteration_hash, /^[0-9a-f]{64}$/);
  const trigger = attemptList.items.find((item) => item.node_id === "start");
  assert.deepEqual(
    (
      await api(
        `${root}/workflow-executions/${execution.id}/node-runs/${trigger.id}/data`,
      )
    ).input.items,
    [],
  );
  const other = await api("/workspaces", {
    name: "Inspector isolation",
    slug: "inspector-other-" + Date.now(),
  });
  await api(detailPath.replace(workspace.id, other.id), undefined, "GET", 404);
  await api(
    `${root}/workflow-executions/${execution.id}/node-runs/1/data`,
    undefined,
    "GET",
    404,
  );
  pass(
    "authenticated frozen inputs, resolved parameters, outputs and workspace/run isolation",
  );

  const page = await context.newPage(),
    errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${origin}/zh-CN${endpoint}/edit`);
  const card = page.locator(".react-flow__node").filter({ hasText: "assign" });
  await card.dblclick();
  let dialog = page.getByRole("dialog");
  await dialog.locator(`section[aria-label="${t("inspector.input")}"]`)
    .getByRole("button", {
      name: `${t("inspector.reference")} ["profile","name"]`,
      exact: true,
    })
    .waitFor();
  const inputPane = dialog.locator(
      `section[aria-label="${t("inspector.input")}"]`,
    ),
    parameters = dialog.locator(
      `section[aria-label="${t("inspector.parameters")}"]`,
    ),
    output = dialog.locator(`section[aria-label="${t("inspector.output")}"]`);
  assert(await inputPane.isVisible());
  assert(await parameters.isVisible());
  assert(await output.isVisible());
  assert((await inputPane.innerText()).includes("Ada"));
  assert((await output.innerText()).includes("Ada"));
  await page.screenshot({
    path: join(directory, "three-pane-desktop.png"),
    fullPage: true,
  });
  pass(
    "double-click opens a real three-pane node editor with execution input and output",
  );

  // Direct field references are shown as a readable path; the saved value
  // remains an expression. Inspect both the user control and persisted graph.
  const field = parameters.getByLabel(t("value"), {exact:true});
  assert.equal(await field.inputValue(), "profile.name");
  await field.focus();
  await inputPane
    .getByRole("button", {
      name: `${t("inspector.reference")} ["profile","name"]`,
      exact: true,
    })
    .click();
  assert.equal(await field.inputValue(), "profile.name");
  const previewResponse = page.waitForResponse(response => response.url().endsWith("/expression-previews"));
  await dialog
    .getByRole("button", { name: t("inspector.resolve"), exact: true })
    .click();
  await previewResponse;
  for (let depth=0; depth<10 && await output.locator("details:not([open]) > summary").count(); depth++)
    await output.locator("details:not([open]) > summary").first().click();
  await output.getByText("Ada", { exact: true }).waitFor();
  const before = await api(`${endpoint}/executions`);
  assert.equal(before.total, 1, "parameter preview creates no execution");
  const savedResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(endpoint) &&
      response.request().method() === "GET",
  );
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await savedResponse;
  row = await api(endpoint);
  assert.equal(
    JSON.parse(row.definition).nodes[1].params.fields[0].value,
    '={{ $json["profile"]["name"] }}',
  );
  assert(await dialog.isVisible(), "saving from the panel keeps it open");
  await dialog
    .getByRole("button", { name: t("inspector.output.execution"), exact: true })
    .click();
  await parameters
    .getByRole("button", { name: t("inspector.settings"), exact: true })
    .click();
  assert(
    await dialog
      .getByText(t("inspector.historical"), { exact: true })
      .isVisible(),
  );
  assert(
    await parameters
      .getByRole("button", { name: t("inspector.pin"), exact: true })
      .isDisabled(),
  );
  pass(
    "click-to-reference edits the real form, parameter preview creates no runs, panel save persists and marks historical data",
  );

  const latest = await run(row, { profile: { name: "Grace" } });
  await dialog
    .getByRole("button", { name: t("inspector.refresh"), exact: true })
    .click();
  await dialog
    .getByLabel(t("inspector.execution"), { exact: true })
    .selectOption(latest.id);
  await output.getByRole("button", {name:`${t("inspector.reference")} ["greeting"]`, exact:true})
    .locator("..").locator("..").getByText("Grace", {exact:true}).waitFor();
  await parameters
    .getByRole("button", { name: t("inspector.pin"), exact: true })
    .click();
  const pinnedSaved = page.waitForResponse(
    (response) =>
      response.url().endsWith(endpoint) &&
      response.request().method() === "GET",
  );
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await pinnedSaved;
  row = await api(endpoint);
  assert.equal(
    JSON.parse(row.definition).pin_data.assign[0].json.greeting,
    "Grace",
  );
  const pinned = await run(row, { profile: { name: "Changed input" } });
  const pinnedNodes = (
    await api(`${root}/workflow-executions/${pinned.id}/node-runs`)
  ).items;
  assert.equal(
    pinnedNodes.find((item) => item.node_id === "assign").state,
    "pinned",
  );
  const final = pinnedNodes.find((item) => item.node_id === "finish");
  assert.equal(
    (
      await api(
        `${root}/workflow-executions/${pinned.id}/node-runs/${final.id}/data`,
      )
    ).output.ports[0].items[0].json.greeting,
    "Grace",
  );
  await parameters
    .getByRole("button", { name: t("inspector.unpin"), exact: true })
    .click();
  await parameters
    .getByLabel(t("inspector.mode"), { exact: true })
    .selectOption("bypass");
  await parameters
    .getByLabel(t("inspector.notes"), { exact: true })
    .fill("Node inspector acceptance");
  const bypassSaved = page.waitForResponse(
    (response) =>
      response.url().endsWith(endpoint) &&
      response.request().method() === "GET",
  );
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await bypassSaved;
  row = await api(endpoint);
  const bypassed = await run(row, { profile: { name: "Bypass" } });
  const bypassNodes = (
    await api(`${root}/workflow-executions/${bypassed.id}/node-runs`)
  ).items;
  assert.equal(
    bypassNodes.find((item) => item.node_id === "assign").state,
    "bypassed",
  );
  assert.equal(
    bypassNodes.find((item) => item.node_id === "finish").state,
    "succeeded",
  );
  await parameters
    .getByLabel(t("inspector.mode"), { exact: true })
    .selectOption("mute");
  const muteSaved = page.waitForResponse(
    (response) =>
      response.url().endsWith(endpoint) &&
      response.request().method() === "GET",
  );
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await muteSaved;
  row = await api(endpoint);
  const muted = await run(row);
  const muteNodes = (
    await api(`${root}/workflow-executions/${muted.id}/node-runs`)
  ).items;
  assert.equal(
    muteNodes.find((item) => item.node_id === "assign").state,
    "skipped",
  );
  assert.equal(
    muteNodes.find((item) => item.node_id === "finish").state,
    "skipped",
  );
  pass(
    "pinned output changes actual manual execution; bypass passes input and mute skips downstream",
  );

  await parameters
    .getByLabel(t("inspector.mode"), { exact: true })
    .selectOption("active");
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForResponse(
    (response) =>
      response.url().endsWith(endpoint) &&
      response.request().method() === "GET",
  );
  row = await api(endpoint);
  await dialog.getByRole("button", { name: t("close"), exact: true }).click();
  await card.click();
  await page.locator(".react-flow__pane").focus();
  await page.keyboard.press("Control+d");
  assert.equal(await page.locator(".react-flow__node").count(), 4);
  await page.keyboard.press("Control+z");
  assert.equal(await page.locator(".react-flow__node").count(), 3);
  await card.click();
  await page.locator(".react-flow__pane").focus();
  await page.keyboard.press("Enter");
  await dialog.waitFor();
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "hidden" });
  await card.click({ button: "right" });
  await page
    .getByRole("menuitem", { name: t("parameters"), exact: true })
    .click();
  await dialog.waitFor();
  pass(
    "keyboard duplication/undo, Enter/Escape and right-click open the full editor without breaking the canvas",
  );

  await page.setViewportSize({ width: 390, height: 844 });
  await dialog
    .getByRole("button", { name: t("inspector.input"), exact: true })
    .click();
  assert(await inputPane.isVisible());
  assert(!(await parameters.isVisible()));
  await dialog
    .getByRole("button", { name: t("inspector.parameters"), exact: true })
    .first()
    .click();
  assert(await parameters.isVisible());
  await dialog
    .getByRole("button", { name: t("inspector.output"), exact: true })
    .click();
  assert(await output.isVisible());
  assert(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth + 1,
    ),
  );
  const bounds = await dialog.boundingBox();
  assert(
    bounds.x >= 0 &&
      bounds.y >= 0 &&
      bounds.x + bounds.width <= 390 &&
      bounds.y + bounds.height <= 844,
  );
  await page.screenshot({
    path: join(directory, "three-pane-mobile.png"),
    fullPage: true,
  });
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto(`${origin}/zh-CN${endpoint}`);
  await card.click();
  await page.getByRole("button", {name:t("inspector.open"), exact:true}).click();
  dialog = page.getByRole("dialog");
  await dialog
    .getByRole("button", { name: t("inspector.resolve"), exact: true })
    .waitFor();
  assert(
    await dialog
      .getByRole("button", { name: t("inspector.resolve"), exact: true })
      .isDisabled(),
  );
  assert.equal(
    await dialog.getByRole("button", { name: "保存", exact: true }).count(),
    0,
  );
  await dialog
    .getByRole("button", { name: t("inspector.settings"), exact: true })
    .click();
  assert(
    await dialog.getByLabel(t("inspector.mode"), { exact: true }).isDisabled(),
  );
  assert.deepEqual(errors, []);
  pass(
    "390px tabbed panes fit the viewport and read-only inspection exposes no parameter mutation",
  );

  const mergeGraph = {
    ...graph,
    nodes: [
      graph.nodes[0],
      node(
        "left",
        "data.set",
        {
          keep_other_fields: false,
          fields: [{ path: "branch", value: "left" }],
        },
        320,
      ),
      node(
        "right",
        "data.set",
        {
          keep_other_fields: false,
          fields: [{ path: "branch", value: "right" }],
        },
        320,
      ),
      node(
        "merge",
        "core.merge",
        { mode: "append", input_count: 2, wait: "all_inputs" },
        640,
      ),
    ],
    connections: [
      link("start", "left"),
      link("start", "right"),
      link("left", "merge", 0, "input"),
      link("right", "merge", 1, "input"),
    ],
  };
  const mergedRow = await api(
    `${root}/workflows`,
    {
      name: "Inspector input source ordering",
      definition: JSON.stringify(mergeGraph),
    },
    "POST",
    201,
  );
  const mergedRun = await run(mergedRow);
  const mergeAttempt = (
    await api(`${root}/workflow-executions/${mergedRun.id}/node-runs`)
  ).items.find((item) => item.node_id === "merge");
  const mergedData = await api(
    `${root}/workflow-executions/${mergedRun.id}/node-runs/${mergeAttempt.id}/data`,
  );
  assert.deepEqual(
    mergedData.input.items.map((item) => item.json.branch),
    ["left", "right"],
  );
  assert.equal(mergedData.input.sources.length, 2);
  pass("multi-input inspection preserves the execution's frozen source order");
  await writeFile(
    join(directory, "acceptance.json"),
    JSON.stringify(
      {
        result: "PASS",
        origin,
        workspace: workspace.id,
        workflow: row.id,
        execution: execution.id,
        checks,
        screenshots: ["three-pane-desktop.png", "three-pane-mobile.png"],
      },
      null,
      2,
    ),
  );
} finally {
  if (browser) {
    const page = browser.contexts()[0]?.pages()[0];
    if (page) await page.screenshot({path:join(directory,"diagnostic.png"), fullPage:true}).catch(() => {});
    await browser.close();
  }
  if (server && server.exitCode === null) {
    server.kill("SIGTERM");
    await new Promise((r) => server.once("exit", r));
  }
  await log.close();
  console.log("Node inspector evidence:", directory);
}
