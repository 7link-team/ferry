import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { generate, targets, checkVersions } from './runner-manifest.mjs';

test('catalog binds each target to its exact bytes and version', () => {
  const dir = mkdtempSync(join(tmpdir(), 'runner-manifest-'));
  try {
    for (const [target, file] of Object.entries(targets)) writeFileSync(join(dir, file), target);
    const catalog = generate(dir, '0.1.0-alpha.12', 'owner/project', 1);
    assert.equal(catalog.version, '0.1.0-alpha.12');
    assert.equal(catalog.schema, 1);
    assert.equal(catalog.protocol, 1);
    assert.equal(Object.keys(catalog.targets).length, 5);
    for (const [target, file] of Object.entries(targets)) {
      assert.equal(catalog.targets[target].size, Buffer.byteLength(target));
      assert.equal(catalog.targets[target].sha256, createHash('sha256').update(target).digest('hex'));
      assert.equal(catalog.targets[target].url, `https://github.com/owner/project/releases/download/v0.1.0-alpha.12/${file}`);
    }
    rmSync(join(dir, targets['x86_64-pc-windows-msvc']));
    assert.throws(() => generate(dir, '0.1.0-alpha.12', 'owner/project', 1));
    writeFileSync(join(dir, targets['x86_64-pc-windows-msvc']), '');
    assert.throws(() => generate(dir, '0.1.0-alpha.12', 'owner/project', 1), /empty/i);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('catalog rejects invalid metadata', () => {
  for (const version of ['../bad', '1.2', 'v1.2.3', '01.2.3'])
    assert.throws(() => generate('.', version, 'owner/project', 1));
  for (const repo of ['owner', '../project', 'https://github.com/owner/project'])
    assert.throws(() => generate('.', '1.2.3', repo, 1));
  for (const protocol of [0, -1, 1.5, NaN])
    assert.throws(() => generate('.', '1.2.3', 'owner/project', protocol));
});

test('release versions must agree before signing', () => {
  checkVersions('1.2.3', '1.2.3', 'v1.2.3');
  checkVersions('1.2.3-alpha.1', '1.2.3-alpha.1');
  assert.throws(() => checkVersions('1.2.3', '1.2.2', 'v1.2.3'));
  assert.throws(() => checkVersions('1.2.3', '1.2.3', 'v1.2.2'));
});
