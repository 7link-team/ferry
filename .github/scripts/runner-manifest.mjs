import { createHash, createPublicKey, verify } from 'node:crypto';
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

export function verifySignature(bytes, signature, publicKey) {
  const publicLines = Buffer.from(publicKey.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
  const keyBytes = Buffer.from(publicLines[1] ?? '', 'base64');
  const lines = Buffer.from(signature.trim(), 'base64').toString('utf8').trim().split(/\r?\n/);
  const signed = Buffer.from(lines[1] ?? '', 'base64');
  if (keyBytes.length !== 42 || signed.length !== 74 || signed.subarray(0, 2).toString() !== 'ED' || !signed.subarray(2, 10).equals(keyBytes.subarray(2, 10))) throw new Error('Invalid signing key or signature');
  const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), keyBytes.subarray(10)]), format: 'der', type: 'spki' });
  if (!verify(null, createHash('blake2b512').update(bytes).digest(), key, signed.subarray(10))) throw new Error('Invalid catalog signature');
  if (!lines[2]?.startsWith('trusted comment: ') || !verify(null, Buffer.concat([signed.subarray(10), Buffer.from(lines[2].slice('trusted comment: '.length))]), key, Buffer.from(lines[3] ?? '', 'base64'))) throw new Error('Invalid trusted comment signature');
}

export function checkVersions(version, cargoVersion, tag) {
  if (version !== cargoVersion || (tag && tag !== `v${version}`)) throw new Error('Package, Cargo and release tag versions must agree');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [, , directory, version, repository] = process.argv;
  const cargo = readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8');
  checkVersions(version, cargo.match(/^version = "([^"]+)"$/m)?.[1], process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : undefined);
  const protocol = Number(readFileSync(new URL('../../crates/protocol/src/lib.rs', import.meta.url), 'utf8').match(/pub const VERSION: u32 = (\d+);/)?.[1]);
  if (process.argv.includes('--verify')) {
    const config = JSON.parse(readFileSync(new URL('../../desktop/tauri.conf.json', import.meta.url), 'utf8'));
    const bytes = readFileSync(resolve(directory, 'runner-update.json'));
    verifySignature(bytes, readFileSync(resolve(directory, 'runner-update.json.sig'), 'utf8'), config.plugins.updater.pubkey);
    if (bytes.toString() !== `${JSON.stringify(generate(directory, version, repository, protocol), null, 2)}\n`) throw new Error('Signed catalog does not match release artifacts');
    console.log('PASS signed runner catalog and release artifacts');
  } else writeFileSync(resolve(directory, 'runner-update.json'), `${JSON.stringify(generate(directory, version, repository, protocol), null, 2)}\n`);
}
