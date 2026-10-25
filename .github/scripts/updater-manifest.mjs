import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export function generate(directory, version, repository) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error("Invalid release version");
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error("Invalid repository");
  const files = readdirSync(directory);
  const targets = {
    "darwin-aarch64": (f) => f === "updater-aarch64-apple-darwin.app.tar.gz",
    "darwin-x86_64": (f) => f === "updater-x86_64-apple-darwin.app.tar.gz",
    "linux-x86_64": (f) => f.endsWith("_amd64.AppImage"),
    "linux-x86_64-deb": (f) => f.endsWith("_amd64.deb"),
    "windows-x86_64": (f) => f.endsWith("_x64-setup.exe"),
  };
  const platforms = {};
  for (const [target, matches] of Object.entries(targets)) {
    const candidates = files.filter(matches);
    if (candidates.length !== 1) throw new Error(`${target}: expected exactly one update artifact, found ${candidates.length}`);
    const file = candidates[0];
    if (!readFileSync(resolve(directory, file)).length) throw new Error(`${file}: empty artifact`);
    const signature = readFileSync(resolve(directory, `${file}.sig`), "utf8").trim();
    const decoded = Buffer.from(signature, "base64").toString("utf8");
    if (!decoded.startsWith("untrusted comment:") || !decoded.includes("trusted comment:")) throw new Error(`${file}: invalid minisign signature`);
    platforms[target] = { signature, url: `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(file)}` };
  }
  return { version, notes: `Desktop update ${version}`, platforms };
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [, , directory, version, repository] = process.argv;
  const manifest = generate(directory, version, repository);
  writeFileSync(resolve(directory, "latest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Update manifest: ${manifest.version}, ${Object.keys(manifest.platforms).join(", ")}`);
}
