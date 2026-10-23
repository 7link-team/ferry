#!/usr/bin/env node
// Checks that a built server image serves the real product, not just that it
// starts: it initializes a fresh data volume, answers /api/status, serves the
// embedded web UI and its client-side routes, and comes back as the same
// installation after the container is replaced over the same volume.
//
// Usage: node .github/scripts/smoke-server-image.mjs <image>
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";

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

const docker = (...args) =>
  execFileSync("docker", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }).trim();

// The image already sets FERRY_DATA_DIR, FERRY_BIND and FERRY_SERVER_MODE; the
// volume is what makes the restart check meaningful.
function start(name) {
  started.push(name);
  docker("run", "-d", "--name", name, "-v", `${volume}:/data`, "-p", "4290:4290", image);
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

try {
  start(names[0]);
  const first = await waitForStatus("first boot");
  console.log(`first boot ok: ${JSON.stringify(first)}`);

  await expectWebUi("/");
  // A client-side route is not a file; the server has to fall back to the shell.
  await expectWebUi("/demo/host/hosts");
  console.log("embedded web UI and client-side routes ok");

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
