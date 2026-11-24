#!/usr/bin/env node
// Removes regenerable local output so the checkout does not grow without bound.
// Usage: npm run clean:local            -> scratch, logs, caches, desktop/web build output
//        npm run clean:local -- --all   -> also the Rust target directory (full rebuild next time)
// Never touches tracked files, node_modules or data/dev-backend.
import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const all = process.argv.includes("--all");
const evidenceDays = 7;

const paths = [
  ".tmp",
  "dist",
  "harness-logs",
  "tsconfig.tsbuildinfo",
  "desktop/gen",
  "desktop/ui-dist",
  "desktop/target",
  "desktop/target-mobile",
  ".github/scripts/__pycache__",
  "scripts/__pycache__",
  ...(all ? ["target"] : []),
];

const removed = [];
const remove = path => {
  if (!existsSync(path)) return;
  rmSync(path, { recursive: true, force: true, maxRetries: 3 });
  removed.push(path);
};

for (const path of paths) remove(join(root, path));
for (const name of readdirSync(root)) if (name.endsWith(".log")) remove(join(root, name));

// Old browser/release evidence kept outside the repository.
const cutoff = Date.now() - evidenceDays * 24 * 60 * 60 * 1000;
const evidence = join(process.env.LOCALAPPDATA ?? join(homedir(), ".local", "share"), "ferry-verification");
if (existsSync(evidence)) {
  for (const name of readdirSync(evidence)) {
    const path = join(evidence, name);
    if (statSync(path).mtimeMs < cutoff) remove(path);
  }
}
for (const name of readdirSync(tmpdir())) {
  if (!/^(conversation-pending|detail-|dialog-layout)/.test(name)) continue;
  const path = join(tmpdir(), name);
  try { if (statSync(path).mtimeMs < cutoff) remove(path); } catch { /* vanished or locked */ }
}

console.log(removed.length ? `removed:\n${removed.join("\n")}` : "nothing to remove");
