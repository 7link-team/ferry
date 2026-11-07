import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { createServer } from "node:net";
import { copyFile, mkdir, mkdtemp, open, readFile, rename, writeFile } from "node:fs/promises";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import assert from "node:assert/strict";

const directory = await mkdtemp(join(tmpdir(), "workflow-integrations-"));
const listener = createServer();
await new Promise((resolve) => listener.listen(0, "127.0.0.1", resolve));
const port = listener.address().port;
await new Promise((resolve) => listener.close(resolve));
const external = process.env.FERRY_TEST_ORIGIN;
if (!external) {
  await mkdir(join(directory, "workflow-extensions"));
  await copyFile("examples/workflow-extensions/public-repository.json", join(directory, "workflow-extensions/public-repository.json"));
}
const origin = external || `http://127.0.0.1:${port}`;
const log = await open(join(directory, "server.log"), "w");
const env = {
  ...process.env,
  FERRY_DATA_DIR: directory,
  FERRY_BIND: `127.0.0.1:${port}`,
  FERRY_SANDBOX_PATH: resolve(
    `target/debug/ferry-sandbox${process.platform === "win32" ? ".exe" : ""}`,
  ),
  FERRY_ARTIFACT_STORE: "local",
};
for (const key of [
  "FERRY_DATABASE_URL",
  "FERRY_NATS_URL",
  "FERRY_CLUSTER_NAMESPACE",
])
  delete env[key];
const launch = () => spawn(
      resolve(
        `target/debug/ferry-server${process.platform === "win32" ? ".exe" : ""}`,
      ),
      [],
      { env, windowsHide: true, stdio: ["ignore", log.fd, log.fd] },
    );
let server = external ? undefined : launch();
async function stopServer() {
  if (!server || server.exitCode !== null) return;
  const stopped = once(server, "exit");
  server.kill("SIGTERM");
  await stopped;
}
async function restartServer() {
  server = launch();
  for (let attempt = 0; attempt < 200; attempt++) {
    assert.equal(server.exitCode, null, "server exited during restart");
    try { if ((await fetch(`${origin}/api/status`)).ok) return; } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.fail("restart timed out");
}
let browser;
const checks = [];
const pass = (name) => {
  checks.push(name);
  console.log("PASS", name);
};
const catalog = JSON.parse(
  await readFile(
    "src/shared/workflow-contracts/workflow-integration-nodes.json",
    "utf8",
  ),
);
const messages = JSON.parse(
  await readFile("src/shared/i18n/locales/workflow-domain.json", "utf8"),
);
const t = (key) => messages[key][1];
const node = (id, type, params, x) => ({
  id,
  name: id,
  type,
  params,
  type_version: 1,
  position: [x, 160],
});
const definition = (params) => ({
  engine: "ferry-graph",
  schema: 1,
  nodes: [
    node(
      "start",
      "core.trigger.manual",
      { input_schema: { type: "object" } },
      0,
    ),
    {
      ...node("repository", "integration.github.repository.get", params, 320),
      name: "Read public repository",
    },
  ],
  connections: [
    {
      from: { node: "start", port: "main", index: 0 },
      to: { node: "repository", port: "main", index: 0 },
    },
  ],
});
try {
  for (let attempt = 0; ; attempt++) {
    try {
      if ((await fetch(`${origin}/api/status`)).ok) break;
    } catch {}
    assert(attempt < 200, "server startup timeout");
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  browser = await chromium.launch({
    channel: process.env.FERRY_BROWSER_CHANNEL || "chrome",
    headless: true,
  });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 960 },
  });
  const headers = { "X-Ferry-Request": "1", "Accept-Language": "zh-CN" };
  async function api(
    path,
    data,
    method = data ? "POST" : "GET",
    expected = 200,
  ) {
    const response = await context.request.fetch(`${origin}/api${path}`, {
      method,
      headers,
      ...(data ? { data } : {}),
    });
    assert.equal(
      response.status(),
      expected,
      `${path}: ${await response.text()}`,
    );
    return response.status() === 204 ? undefined : response.json();
  }
  if (external) {
    const access = JSON.parse(
      await readFile(process.env.FERRY_TEST_ACCESS_FILE, "utf8"),
    );
    await api("/sessions", { email: access.email, password: access.password });
  } else
    await api("/setup", {
      name: "Integration acceptance",
      email: "integrations@example.invalid",
      password: "temporary-integration-password",
      token: (await readFile(join(directory, "setup-token"), "utf8")).trim(),
    });
  const workspace = await api("/workspaces", {
    name: "Integration acceptance",
    slug: `integrations-${Date.now()}`,
  });
  const root = `/workspaces/${workspace.id}`;
  const runtimeCatalog = await api(`${root}/workflow-node-catalog`);
  const extension = runtimeCatalog.nodes.find(node => node.type === "integration.ext.public_repository.v1.read");
  if (!external) assert(extension, "operator-installed package is loaded without rebuilding");
  async function run(row, expected, inputs = {}) {
    const started = await api(
      `${root}/workflows/${row.id}/executions`,
      {
        mode: "manual",
        revision: "draft",
        version: row.version,
        entry_node_id: "start",
        inputs,
      },
      "POST",
      201,
    );
    let result;
    for (let attempt = 0; attempt < 450; attempt++) {
      result = await api(`${root}/workflow-executions/${started.id}`);
      if (["succeeded", "failed", "cancelled"].includes(result.state)) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.equal(result.state, expected, JSON.stringify(result));
    return result;
  }
  const publicParams = {
    credential_id: "0",
    owner: "octocat",
    repo: "Hello-World",
  };
  let extensionRow;
  if (extension) {
    const graph = definition(publicParams);
    graph.nodes[1].type = extension.type;
    extensionRow = await api(`${root}/workflows`, {name:"Runtime extension acceptance", definition:JSON.stringify(graph)}, "POST", 201);
    const execution = await run(extensionRow, "succeeded");
    const attempts = await api(`${root}/workflow-executions/${execution.id}/node-runs`);
    const attempt = attempts.items.find(node => node.node_id === "repository");
    const output = await api(`${root}/workflow-executions/${execution.id}/node-runs/${attempt.id}/output`);
    assert.equal(output.ports[0].items[0].json.full_name, "octocat/Hello-World");
    const wrongProfile = await api(`${root}/credentials`, {name:"Local extension isolation", scope:"integration", kind:"token", username:"github", secret:"LOCAL_EXTENSION_ISOLATION"}, "POST", 201);
    const isolated = definition({...publicParams, credential_id:wrongProfile.id});
    isolated.nodes[1].type = extension.type;
    const isolatedRow = await api(`${root}/workflows`, {name:"Extension isolation", definition:JSON.stringify(isolated)}, "POST", 201);
    await run(isolatedRow, "failed");
    pass("runtime package registers, saves and performs a real HTTP execution; foreign credentials fail");
  }
  let row = await api(
    `${root}/workflows`,
    {
      name: "GitHub public repository · integration acceptance",
      definition: JSON.stringify(definition(publicParams)),
    },
    "POST",
    201,
  );
  const execution = await run(row, "succeeded");
  const attempts = await api(
    `${root}/workflow-executions/${execution.id}/node-runs`,
  );
  const attempt = attempts.items.find((node) => node.node_id === "repository");
  const output = await api(
    `${root}/workflow-executions/${execution.id}/node-runs/${attempt.id}/output`,
  );
  assert.equal(output.ports[0].items[0].json.full_name, "octocat/Hello-World");
  pass(
    "saved workflow performs a real public GitHub request and persists its actual output",
  );
  const expressionGraph = definition({
    credential_id: "0",
    owner: "={{ $json.owner }}",
    repo: "={{ $json.repo }}",
  });
  const expression = await api(
    `${root}/workflows`,
    {
      name: "Expression integration",
      definition: JSON.stringify(expressionGraph),
    },
    "POST",
    201,
  );
  await run(expression, "succeeded", { owner: "octocat", repo: "Hello-World" });
  pass(
    "integration parameters resolve input expressions through the existing sandbox",
  );
  const missingRepository = await api(
    `${root}/workflows`,
    {
      name: "GitHub upstream error acceptance",
      definition: JSON.stringify(
        definition({
          ...publicParams,
          repo: "ferry-acceptance-missing-" + Date.now(),
        }),
      ),
    },
    "POST",
    201,
  );
  const rejected = await run(missingRepository, "failed");
  assert.equal(rejected.end_reason, "integration_remote");
  pass(
    "real GitHub errors terminate the execution with an integration failure reason",
  );

  await api(
    `${root}/credentials`,
    {
      name: "Wrong kind",
      scope: "integration",
      kind: "password",
      username: "slack",
      secret: "LOCAL_TEST_ONLY",
    },
    "POST",
    400,
  );
  await api(
    `${root}/credentials`,
    {
      name: "Wrong app",
      scope: "integration",
      kind: "token",
      username: "unknown-app",
      secret: "LOCAL_TEST_ONLY",
    },
    "POST",
    400,
  );
  const createdCredential = await api(
    `${root}/credentials`,
    {
      name: "Local acceptance GitHub token",
      scope: "integration",
      kind: "token",
      username: "github",
      secret: "LOCAL_TEST_ONLY",
    },
    "POST",
    201,
  );
  const credential = await api(`${root}/credentials/${createdCredential.id}`);
  assert(!JSON.stringify(credential).includes("LOCAL_TEST_ONLY"));
  await api(
    `${root}/credentials/${credential.id}`,
    {
      name: credential.name,
      scope: "integration",
      kind: "token",
      username: "slack",
      version: credential.version,
    },
    "PATCH",
    409,
  );
  await api(
    `${root}/workflow-integration-options`,
    {
      node_type: "integration.slack.message.create",
      credential_id: credential.id,
    },
    "POST",
    400,
  );
  await api(
    `${root}/workflow-integration-options`,
    {
      node_type: "integration.slack.channel.list",
      credential_id: credential.id,
    },
    "POST",
    400,
  );
  pass(
    "credential kind/profile isolation, immutable application and read-only lookup restrictions",
  );

  const wrongProfile = definition(publicParams);
  wrongProfile.nodes[1] = node(
    "slack_notification",
    "integration.slack.message.create",
    { credential_id: credential.id, channel: "C_TEST", text: "MUST_NOT_SEND" },
    320,
  );
  wrongProfile.connections[0].to.node = "slack_notification";
  const blocked = await api(
    `${root}/workflows`,
    {
      name: "Wrong profile blocked before send",
      definition: JSON.stringify(wrongProfile),
    },
    "POST",
    201,
  );
  assert.equal(
    (await run(blocked, "failed")).end_reason,
    "integration_invalid",
  );
  pass(
    "wrong-provider credentials fail before any message or write request can be sent",
  );

  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  if (extensionRow) {
    await page.goto(`${origin}/zh-CN${root}/workflows/${extensionRow.id}/edit`);
    await page.locator(".react-flow__node").filter({hasText:"Read public repository"}).click();
    await page.getByLabel("Repository name", {exact:true}).fill("Spoon-Knife");
    const saved = page.waitForResponse(response => response.url().endsWith(`/workflows/${extensionRow.id}`) && response.request().method() === "GET");
    await page.getByRole("button", {name:"保存", exact:true}).click();
    await saved;
    extensionRow = await api(`${root}/workflows/${extensionRow.id}`);
    assert.equal(JSON.parse(extensionRow.definition).nodes[1].params.repo, "Spoon-Knife");
    await page.reload();
    await page.locator(".react-flow__node").filter({hasText:"Read public repository"}).click();
    assert.equal(await page.getByLabel("Repository name", {exact:true}).inputValue(), "Spoon-Knife");
    await page.getByRole("button", {name:t("add"), exact:true}).click();
    const picker = page.getByRole("dialog");
    await picker.getByLabel(t("search"), {exact:true}).fill("extension");
    await picker.getByRole("button", {name:"Public repository extension v1 · Repository · Read", exact:true}).waitFor();
    await picker.getByRole("button", {name:"Public repository extension v1 · Repository · Read", exact:true}).waitFor();
    await picker.getByRole("button", {name:t("close"), exact:true}).click();
    await run(extensionRow, "succeeded");
    await page.screenshot({path:join(directory,"extension-parameters.png"),fullPage:true});
    pass("installed package appears in node search and ordinary forms, saves/reopens and executes without compilation");
  }
  const endpoint = `${root}/workflows/${row.id}`;
  await page.goto(`${origin}/zh-CN${endpoint}/edit`);
  const card = page
    .locator(".react-flow__node")
    .filter({ hasText: "Read public repository" });
  await card.click();
  assert.equal(
    await page.getByLabel(t("integration.owner"), { exact: true }).inputValue(),
    "octocat",
  );
  assert.equal(
    await page
      .getByLabel(t("integration.repository"), { exact: true })
      .inputValue(),
    "Hello-World",
  );
  await page.getByRole("option", { name: t("integration.publicAccess"), exact: true }).waitFor({state:"attached"});
  assert.equal(await page.locator("details[open]").count(), 0);
  await page.getByRole("button", { name: t("add"), exact: true }).click();
  let dialog = page.getByRole("dialog");
  await dialog.getByLabel(t("search"), { exact: true }).fill("git");
  assert(await dialog.getByRole("button",{name:/^GitHub · /}).count(),"GitHub operations match git search");
  assert(await dialog.getByRole("button",{name:/^GitLab · /}).count(),"GitLab operations match git search");
  for (const spec of catalog.nodes.filter(
    (node) => node.ui.application === "github",
  )) {
    await dialog
      .getByRole("button", {
        name: `GitHub · ${t(spec.ui.resource_key)} · ${t(spec.ui.operation_key)}`,
        exact: true,
      })
      .waitFor();
  }
  await dialog.getByRole("button", { name: t("close"), exact: true }).click();
  pass(
    "application search, all declared GitHub operations and ordinary parameters are visible without JSON",
  );

  await page
    .getByLabel(t("integration.repository"), { exact: true })
    .fill("Spoon-Knife");
  const savedResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith(endpoint) &&
      response.request().method() === "GET",
  );
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await savedResponse;
  row = await api(endpoint);
  assert.deepEqual(JSON.parse(row.definition).nodes[1].params, {
    ...publicParams,
    repo: "Spoon-Knife",
  });
  await page.reload();
  await card.click();
  assert.equal(
    await page.getByLabel(t("integration.owner"), { exact: true }).inputValue(),
    "octocat",
  );
  assert.equal(
    await page
      .getByLabel(t("integration.repository"), { exact: true })
      .inputValue(),
    "Spoon-Knife",
  );
  await run(row, "succeeded");
  await page.screenshot({
    path: join(directory, "github-parameters.png"),
    fullPage: true,
  });
  pass("ordinary form save and reopen retain exact integration parameters");

  // A local-only credential is created for UI testing. No Slack request is executed.
  const slack = catalog.nodes.find(
    (node) => node.type === "integration.slack.message.create",
  );
  const slackGraph = {
    engine: "ferry-graph",
    schema: 1,
    nodes: [
      node(
        "notification",
        slack.type,
        { credential_id: "0", channel: "C_TEST", text: "Local preview only" },
        0,
      ),
    ],
    connections: [],
  };
  const form = await api(
    `${root}/workflows`,
    {
      name: "Slack connection form acceptance",
      definition: JSON.stringify(slackGraph),
    },
    "POST",
    201,
  );
  await page.goto(`${origin}/zh-CN${root}/workflows/${form.id}/edit`);
  await page.locator(".react-flow__node").click();
  await page
    .getByRole("button", {
      name: t("integration.createCredential"),
      exact: true,
    })
    .click();
  dialog = page.getByRole("dialog");
  await dialog
    .getByLabel(t("integration.name"), { exact: true })
    .fill("Local Slack UI credential");
  await dialog
    .getByLabel(t("integration.token"), { exact: true })
    .fill("LOCAL_UI_TOKEN_NOT_A_REAL_SECRET");
  await dialog.getByRole("button", { name: "创建", exact: true }).click();
  await dialog.waitFor({ state: "hidden" });
  const id = await page
    .getByLabel(t("integration.credential"), { exact: true })
    .inputValue();
  assert.notEqual(id, "0");
  assert.equal(
    await page
      .getByLabel(t("integration.credential"), { exact: true })
      .locator("option:checked")
      .textContent(),
    "Local Slack UI credential",
  );
  await page
    .getByLabel(t("integration.message"), { exact: true })
    .fill("={{ $json.message }}");
  const slackSaved = page.waitForResponse(
    (response) =>
      response.url().endsWith(`/workflows/${form.id}`) &&
      response.request().method() === "GET",
  );
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await slackSaved;
  const saved = await api(`${root}/workflows/${form.id}`);
  assert(!saved.definition.includes("LOCAL_UI_TOKEN"));
  assert.equal(JSON.parse(saved.definition).nodes[0].params.credential_id, id);
  await page.screenshot({
    path: join(directory, "slack-parameters.png"),
    fullPage: true,
  });
  pass(
    "real credential creation through the UI binds Slack and saves only its ID, never the token",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  const mobileMessage = page.getByLabel(t("integration.message"), {
    exact: true,
  });
  await mobileMessage.scrollIntoViewIfNeeded();
  assert(await mobileMessage.isVisible());
  assert.equal(await mobileMessage.inputValue(), "={{ $json.message }}");
  assert(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth + 1,
    ),
  );
  await page.screenshot({
    path: join(directory, "mobile.png"),
    fullPage: true,
  });
  assert.deepEqual(errors, []);
  pass("mobile form has no horizontal overflow and no browser exceptions");

  if (!external) {
    const packagePath = join(directory, "workflow-extensions/public-repository.json");
    const originalPackage = await readFile(packagePath, "utf8");
    const originalLock = await readFile(join(directory, "workflow-extensions.lock.json"), "utf8");
    await stopServer();
    const changed = JSON.parse(originalPackage);
    changed.nodes[0].http.path = "/users/{owner}";
    await writeFile(packagePath, JSON.stringify(changed));
    server = launch();
    const [code] = await once(server, "exit");
    assert.notEqual(code, 0, "changing registered package content must prevent startup");
    assert.equal(await readFile(join(directory, "workflow-extensions.lock.json"), "utf8"), originalLock);
    await rename(packagePath, `${packagePath}.disabled`);
    await restartServer();
    assert.equal((await api(`${root}/workflow-node-catalog`)).packages.length, 0);
    const retained = await api(`${root}/workflows/${extensionRow.id}`);
    assert.equal(retained.definition, extensionRow.definition);
    await stopServer();
    await writeFile(`${packagePath}.disabled`, originalPackage);
    await rename(`${packagePath}.disabled`, packagePath);
    await restartServer();
    assert.equal((await api(`${root}/workflow-node-catalog`)).packages.length, 1);
    await run(extensionRow, "succeeded");
    pass("same binary rejects modified versions, disables packages without rewriting workflows, and restores execution after re-enabling");
  }

  await writeFile(
    join(directory, "acceptance.json"),
    JSON.stringify(
      {
        result: "PASS",
        checks,
        origin,
        workspace: workspace.id,
        workflow: row.id,
        extension_workflow: extensionRow?.id,
        slack_workflow: form.id,
        execution: execution.id,
        liveProvider: "GitHub public repository GET only",
        upstreamWriteAccounts:
          "not exercised; no real Slack/GitLab tokens provided",
        screenshots: [
          "github-parameters.png",
          "slack-parameters.png",
          "mobile.png",
        ],
      },
      null,
      2,
    ),
  );
} finally {
  if (browser) await browser.close();
  await stopServer();
  await log.close();
  console.log("Integration acceptance evidence:", directory);
}
