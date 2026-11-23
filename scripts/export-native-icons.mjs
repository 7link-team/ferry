// Native packages derive their artwork from the same master as the Web UI.
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdtempSync, readFileSync, writeFileSync, readdirSync, existsSync, copyFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const icons = join(root, 'desktop/icons');
const args = process.argv.slice(2);
function files(directory, prefix = '') {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const path = join(prefix, entry.name);
    return entry.isDirectory() ? files(join(directory, entry.name), path) : [path];
  });
}
if (args[0] === '--sync') {
  const platform = args[1];
  if (!['android', 'ios'].includes(platform)) throw new Error('Expected --sync android or --sync ios');
  const source = join(icons, platform);
  const target = platform === 'android' ? join(root, 'desktop/gen/android/app/src/main/res')
    : join(root, 'desktop/gen/apple/Assets.xcassets/AppIcon.appiconset');
  if (!existsSync(target)) throw new Error(`Initialize the native project before syncing ${platform} icons`);
  if (platform === 'ios') {
    const manifest = JSON.parse(readFileSync(join(target, 'Contents.json'), 'utf8'));
    for (const image of manifest.images) {
      if (image.filename && (!files(source).includes(image.filename))) throw new Error(`Missing iOS icon ${image.filename}`);
    }
  }
  for (const path of files(source)) {
    mkdirSync(dirname(join(target, path)), { recursive: true });
    copyFileSync(join(source, path), join(target, path));
  }
  console.log(`Synced ${platform} icons from the shared brand master`);
} else {
  if (args.length && (args.length !== 1 || args[0] !== '--check')) throw new Error('Expected no arguments or --check');
  const check = args[0] === '--check';
  // Tauri redirects mobile exports into existing native projects when the
  // destination is the project's icons directory. Isolate generation so source
  // exports are identical before and after native project initialization.
  const temporary = mkdtempSync(join(tmpdir(), 'native-brand-icons-'));
  try {
    const result = spawnSync(process.execPath, [require.resolve('@tauri-apps/cli/tauri.js'), 'icon',
      join(root, 'public/brand/logo.png'), '--output', temporary, '--ios-color', '#ffffff'],
    { cwd: root, windowsHide: true, encoding: 'utf8' });
    if (result.status !== 0) throw new Error(result.stderr || result.stdout || result.error?.message);
    // The ICNS encoder emits its HashMap entries in random order. Canonicalize
    // container order without changing any encoded image or size metadata.
    const icns = join(temporary, 'icon.icns');
    const bytes = readFileSync(icns), chunks = [];
    for (let offset = 8; offset < bytes.length;) {
      const size = bytes.readUInt32BE(offset + 4);
      if (size < 8 || offset + size > bytes.length) throw new Error('Invalid generated ICNS');
      chunks.push(bytes.subarray(offset, offset + size)); offset += size;
    }
    chunks.sort(Buffer.compare);
    writeFileSync(icns, Buffer.concat([bytes.subarray(0, 8), ...chunks]));
    // Retain generated inputs in the existing CI diagnostics artifact when a
    // platform's encoder differs; local checks produce no extra files.
    if (check && process.env.NATIVE_ICON_DIAGNOSTICS_DIR) {
      const diagnostic = resolve(root, process.env.NATIVE_ICON_DIAGNOSTICS_DIR);
      for (const path of files(temporary)) {
        mkdirSync(dirname(join(diagnostic, path)), { recursive: true });
        copyFileSync(join(temporary, path), join(diagnostic, path));
      }
    }
    if (check) {
      for (const path of files(temporary)) {
        if (!existsSync(join(icons, path)) || !readFileSync(join(icons, path)).equals(readFileSync(join(temporary, path)))) {
          const message = `Native icon is stale: ${path}. Run npm run brand:icons`;
          console.error(`::error::${message}`);
          throw new Error(message);
        }
      }
    } else {
      for (const path of files(temporary)) {
        mkdirSync(dirname(join(icons, path)), { recursive: true });
        copyFileSync(join(temporary, path), join(icons, path));
      }
    }
    console.log(check ? 'Native icons match the shared Web brand master' : 'Generated native icons from the shared Web brand master');
  } finally {
    if (temporary) {
      const child = relative(resolve(tmpdir()), resolve(temporary));
      if (!child || child.startsWith('..') || isAbsolute(child) || !child.startsWith('native-brand-icons-')) {
        throw new Error('Refusing to remove an unexpected temporary icon directory');
      }
      rmSync(temporary, { recursive: true, force: true });
    }
  }
}
