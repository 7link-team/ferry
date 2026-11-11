#!/usr/bin/env node
// Builds marketplace/index.json (schemaVersion 2) from marketplace/apps/*. Works on Windows, macOS and Linux.
// Package sources are YAML (like Helm charts / Nomad packs); `.json` is still accepted. The bundle stays JSON for the server.
import { readdirSync, readFileSync, writeFileSync, existsSync, statSync } from "node:fs";
import { join, dirname, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { parse as parseYaml } from "yaml";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "marketplace");
const LIMITS = { packages: 500, versions: 20, files: 20, fileBytes: 256 * 1024, iconBytes: 64 * 1024, readmeBytes: 64 * 1024, bundleBytes: 8 * 1024 * 1024 };
const KEY = /^[A-Za-z0-9._-]{1,100}$/;
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
const fail = message => { console.error(`build-marketplace-index: ${message}`); process.exit(1); };
const parseFile = path => { try { const text = readFileSync(path, "utf8"); return extname(path) === ".json" ? JSON.parse(text) : parseYaml(text, { prettyErrors: true }); } catch (error) { fail(`${path}: ${error.message}`); } };
/** `<name>.yaml`, `<name>.yml` or `<name>.json`; more than one is ambiguous. Returns `fallback` when absent (or fails when required). */
const doc = (dir, name, fallback) => {
  const found = [".yaml", ".yml", ".json"].map(ext => join(dir, name + ext)).filter(existsSync);
  if (found.length > 1) fail(`${dir}: keep only one of ${found.map(p => p.slice(dir.length + 1)).join(", ")}`);
  if (found.length === 0) { if (fallback === undefined) fail(`${dir}: missing ${name}.yaml`); return fallback; }
  return parseFile(found[0]);
};
const dirs = path => existsSync(path) ? readdirSync(path).filter(name => statSync(join(path, name)).isDirectory()).sort() : [];
const compare = (a, b) => { const pa = a.split(/[.-]/).map(Number), pb = b.split(/[.-]/).map(Number); for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] - pb[i]; return 0; };

const packages = [];
for (const key of dirs(join(root, "apps"))) {
  if (!KEY.test(key)) fail(`invalid package key ${key}`);
  const base = join(root, "apps", key);
  const meta = doc(base, "package");
  let icon = "";
  for (const name of ["icon.svg", "icon.png"]) {
    const path = join(base, name);
    if (!existsSync(path)) continue;
    const bytes = readFileSync(path);
    if (bytes.length > LIMITS.iconBytes) fail(`${path} exceeds 64 KiB`);
    icon = `data:${extname(name) === ".svg" ? "image/svg+xml" : "image/png"};base64,${bytes.toString("base64")}`;
    break;
  }
  const versions = dirs(base).filter(v => SEMVER.test(v)).sort(compare).map(version => {
    const dir = join(base, version);
    const files = dirs(dir).includes("files") ? readdirSync(join(dir, "files")).sort().map(name => {
      if (!KEY.test(name) || name.length > 57) fail(`invalid file name ${name} (1–57 of A-Z a-z 0-9 . _ -)`);
      const content = readFileSync(join(dir, "files", name), "utf8");
      if (Buffer.byteLength(content) > LIMITS.fileBytes) fail(`${name} exceeds 256 KiB`);
      return { name, content, parameters: [] };
    }) : [];
    if (files.length > LIMITS.files) fail(`${key}@${version} has more than ${LIMITS.files} files`);
    const readmePath = join(dir, "README.md");
    const readme = existsSync(readmePath) ? readFileSync(readmePath, "utf8") : "";
    if (Buffer.byteLength(readme) > LIMITS.readmeBytes) fail(`${key}@${version} README exceeds 64 KiB`);
    const entry = { version, blueprint: doc(dir, "blueprint"), parameters: doc(dir, "parameters", []), files, readme };
    const verified = doc(dir, "verified", null);
    if (verified !== null) entry.verified = verified;
    const requires = doc(dir, "requires", null);
    if (requires !== null) entry.requires = requires;
    return entry;
  });
  if (versions.length === 0 || versions.length > LIMITS.versions) fail(`${key} needs 1–${LIMITS.versions} versions`);
  packages.push({ key, kind: "app", meta, icon, versions });
}
if (packages.length > LIMITS.packages) fail("too many packages");
// No timestamp: identical sources produce an identical file, so CI can detect a stale index.
const text = JSON.stringify({ schemaVersion: 2, packages }, null, 2) + "\n";
if (Buffer.byteLength(text) > LIMITS.bundleBytes) fail("bundle exceeds 8 MiB");
const out = join(root, "index.json");
if (process.argv.includes("--check")) {
  if (!existsSync(out) || readFileSync(out, "utf8").replace(/\r\n/g, "\n") !== text) fail("marketplace/index.json is stale; run node scripts/build-marketplace-index.mjs");
  console.log("marketplace/index.json is up to date");
} else {
  writeFileSync(out, text);
  console.log(`wrote ${out} (${packages.length} packages)`);
}
