import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export const targets = {
  'x86_64-unknown-linux-gnu': 'ferry-runner-x86_64-unknown-linux-gnu',
  'aarch64-unknown-linux-gnu': 'ferry-runner-aarch64-unknown-linux-gnu',
  'x86_64-apple-darwin': 'ferry-runner-x86_64-apple-darwin',
  'aarch64-apple-darwin': 'ferry-runner-aarch64-apple-darwin',
  'x86_64-pc-windows-msvc': 'ferry-runner-x86_64-pc-windows-msvc.exe',
};

export function generate(directory, version, repository, protocol) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(version)) throw new Error('Invalid release version');
  if (!/^[\w-]+\/[\w.-]+$/.test(repository) || repository.endsWith('/..')) throw new Error('Invalid repository');
  if (!Number.isSafeInteger(protocol) || protocol < 1) throw new Error('Invalid protocol');
  const catalog = { schema: 1, protocol, version, targets: {} };
  for (const [target, file] of Object.entries(targets)) {
    const bytes = readFileSync(resolve(directory, file));
    if (!bytes.length) throw new Error(`${file}: empty artifact`);
    catalog.targets[target] = {
      url: `https://github.com/${repository}/releases/download/v${version}/${file}`,
      size: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    };
  }
  return catalog;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [, , directory, version, repository] = process.argv;
  const protocol = Number(readFileSync(new URL('../../crates/protocol/src/lib.rs', import.meta.url), 'utf8').match(/pub const VERSION: u32 = (\d+);/)?.[1]);
  writeFileSync(resolve(directory, 'runner-update.json'), `${JSON.stringify(generate(directory, version, repository, protocol), null, 2)}\n`);
}
