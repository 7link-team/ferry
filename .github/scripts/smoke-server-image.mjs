#!/usr/bin/env node
// Checks that a built server image serves the real product, not just that it
// starts: it initializes a fresh data volume, answers /api/status, serves the
// embedded web UI and its client-side routes, and comes back as the same
// installation after the container is replaced over the same volume.
//
// Usage: node .github/scripts/smoke-server-image.mjs <image>
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { fileURLToPath } from "node:url";

const image = process.argv[2];
if (!image) {
  console.error("usage: smoke-server-image.mjs <image>");
  process.exit(1);
}

const id = randomUUID().slice(0, 8);
const volume = `ferry-smoke-${id}`;
const names = [`${volume}-first`, `${volume}-second`];
const started = [];
const base = "http://127.0.0.1:4290";
const sandboxPolicy = fileURLToPath(new URL("../../sandbox-seccomp.json", import.meta.url));

const docker = (...args) =>
  execFileSync("docker", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }).trim();

// The image already sets FERRY_DATA_DIR, FERRY_BIND and FERRY_SERVER_MODE; the
// volume is what makes the restart check meaningful.
function start(name) {
  started.push(name);
  docker("run", "-d", "--name", name, "--security-opt", "no-new-privileges:true", "--security-opt", `seccomp=${sandboxPolicy}`, "-v", `${volume}:/data`, "-p", "4290:4290", image);
}

async function waitForStatus(what) {
  const deadline = Date.now() + 120_000;
  for (;;) {
    try {
      const response = await fetch(`${base}/api/status`);
      if (response.ok) {
        const body = await response.json();
        if (body.ok === true) return body;
      }
    } catch {
      // Not listening yet.
    }
    if (Date.now() > deadline) throw new Error(`${what}: /api/status never answered`);
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}

async function expectWebUi(path) {
  const response = await fetch(`${base}${path}`);
  const body = await response.text();
  if (!response.ok) throw new Error(`GET ${path} answered ${response.status}`);
  // With an unbuilt dist/ the server answers a plain-text hint instead, so this
  // is what proves the web UI is really embedded in the image.
  if (!body.includes('<div id="root">')) {
    throw new Error(`GET ${path} did not serve the web UI: ${body.slice(0, 200)}`);
  }
}

async function expectStaticFile(path, type) {
  const response = await fetch(`${base}${path}`);
  const contentType = response.headers.get("content-type") ?? "";
  // An unknown path falls back to the HTML shell, so status 200 alone proves nothing.
  if (!response.ok || !contentType.startsWith(type)) {
    throw new Error(`GET ${path} answered ${response.status} ${contentType}, expected ${type}`);
  }
}

async function expectExpressionWorkflow() {
  const token = docker("exec", names[0], "cat", "/data/setup-token");
  const setup = await fetch(`${base}/api/setup`, {
    method: "POST", headers: { "Content-Type": "application/json", "X-Ferry-Request": "1" },
    body: JSON.stringify({ token, name: "Sandbox acceptance", email: "sandbox@example.invalid", password: randomUUID() }),
  });
  if (!setup.ok) throw new Error(`Setup failed: ${setup.status}`);
  const cookie = setup.headers.getSetCookie().map(value => value.split(";")[0]).join("; ");
  const api = async (path, body) => {
    const response = await fetch(`${base}/api${path}`, {
      method: body === undefined ? "GET" : "POST",
      headers: { Cookie: cookie, "Content-Type": "application/json", "X-Ferry-Request": "1" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    if (!response.ok) throw new Error(`${path}: ${response.status} ${await response.text()}`);
    return response.json();
  };
  const workspace = await api("/workspaces", { name: "Sandbox acceptance", slug: "sandbox-acceptance" });
  const scope = `/workspaces/${workspace.id}`;
  const definition = {
    engine: "ferry-graph", schema: 1, settings: { timezone: "UTC" },
    nodes: [
      { id: "start", name: "Start", type: "core.trigger.manual", type_version: 1, position: [0, 0], params: { input_schema: { type: "object" } } },
      { id: "calculate", name: "Calculate", type: "data.set", type_version: 1, position: [260, 0], params: { fields: [{ path: "answer", value: "={{ 1 + 1 }}" }], keep_other_fields: true } },
    ],
    connections: [{ from: { node: "start", port: "main", index: 0 }, to: { node: "calculate", port: "main", index: 0 } }],
  };
  const workflow = await api(`${scope}/workflows`, { name: "Expression acceptance", definition: JSON.stringify(definition) });
  const saved = await api(`${scope}/workflows/${workflow.id}`);
  const run = await api(`${scope}/workflows/${workflow.id}/executions`, { mode: "manual", revision: "draft", version: saved.version, entry_node_id: "start", inputs: {} });
  let finished;
  for (let attempt = 0; attempt < 100; attempt++) {
    finished = await api(`${scope}/workflow-executions/${run.id}`);
    if (["succeeded", "failed", "cancelled"].includes(finished.state)) break;
    await new Promise(resolve => setTimeout(resolve, 300));
  }
  if (finished.state !== "succeeded") throw new Error(`Expression execution did not succeed: ${JSON.stringify(finished)}`);
  const nodes = await api(`${scope}/workflow-executions/${run.id}/node-runs`);
  const calculate = nodes.items.find(node => node.node_id === "calculate");
  if (!calculate) throw new Error("Expression node execution is missing");
  const output = await api(`${scope}/workflow-executions/${run.id}/node-runs/${calculate.id}/output`);
  if (!output.ports.some(port => port.items.some(item => item.json.answer === 2))) throw new Error("Expression node did not produce answer=2");
  console.log("expression workflow saved and executed through the real authenticated API");
}

try {
  start(names[0]);
  const first = await waitForStatus("first boot");
  console.log(`first boot ok: ${JSON.stringify(first)}`);

  await expectWebUi("/");
  // A client-side route is not a file; the server has to fall back to the shell.
  await expectWebUi("/demo/host/hosts");
  console.log("embedded web UI and client-side routes ok");
  // The brand logo and favicons come from public/ and are referenced by index.html.
  for (const path of ["/brand/logo-32.png", "/brand/logo-16.png", "/brand/logo-180.png"]) {
    await expectStaticFile(path, "image/png");
  }
  await expectStaticFile("/brand/logo.ico", "image/");
  console.log("brand logo and favicons ok");
  await expectExpressionWorkflow();

  docker("stop", names[0]);
  start(names[1]);
  const second = await waitForStatus("restart");
  // Equal installation identities prove the second container really read the
  // first one's volume; a reinitialized data directory would report a new one.
  if (first.instance_id && second.instance_id !== first.instance_id) {
    throw new Error(`the restart lost its data: ${first.instance_id} -> ${second.instance_id}`);
  }
  await expectWebUi("/");
  console.log("restart kept the data volume");
} catch (error) {
  for (const name of started) {
    console.error(`---- docker logs ${name} ----`);
    try {
      console.error(docker("logs", name));
    } catch {
      // The container died before Docker could hand back its log.
    }
  }
  console.error(String(error));
  process.exitCode = 1;
} finally {
  for (const name of names) {
    try {
      docker("rm", "-f", name);
    } catch {
      // Already gone.
    }
  }
  try {
    docker("volume", "rm", volume);
  } catch {
    // Already gone.
  }
}
