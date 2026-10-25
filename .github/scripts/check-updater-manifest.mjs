import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { generate } from './updater-manifest.mjs';

test('manifest maps signed artifacts, escapes URLs and rejects incomplete sets', () => {
  const dir = mkdtempSync(join(tmpdir(), 'updater-manifest-'));
  const files = ['updater-aarch64-apple-darwin.app.tar.gz', 'updater-x86_64-apple-darwin.app.tar.gz', 'Desktop App_amd64.AppImage', 'Desktop App_x64-setup.exe'];
  const signature = Buffer.from('untrusted comment: fixture\nfixture\ntrusted comment: fixture\nfixture').toString('base64');
  try {
    for (const file of files) { writeFileSync(join(dir, file), 'payload'); writeFileSync(join(dir, file+'.sig'), signature); }
    const result = generate(dir, '0.1.0-alpha.2', 'example/app');
    assert.equal(Object.keys(result.platforms).length, 4);
    assert.equal(result.platforms['windows-x86_64'].signature, signature);
    assert.match(result.platforms['windows-x86_64'].url, /Desktop%20App_x64-setup.exe$/);
    assert.throws(() => generate(dir, 'bad', 'example/app'), /version/);
    assert.throws(() => generate(dir, '0.1.0', '../example/app'), /repository/);
    writeFileSync(join(dir, files[0]+'.sig'), 'bad');
    assert.throws(() => generate(dir, '0.1.0', 'example/app'), /signature/);
    writeFileSync(join(dir, files[0]+'.sig'), signature);
    writeFileSync(join(dir, files[0]), '');
    assert.throws(() => generate(dir, '0.1.0', 'example/app'), /empty/);
    rmSync(join(dir, files[0]));
    assert.throws(() => generate(dir, '0.1.0', 'example/app'), /exactly one/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
