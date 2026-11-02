import { chromium } from 'playwright';
import { existsSync } from 'node:fs';
import { mkdir, readFile, stat } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// Run from any directory. No network, installations or browser downloads.
const root = dirname(fileURLToPath(import.meta.url));
const output = resolve(root, '../images');
const pages = [
  '01-workflow-list', '02-workflow-editor', '03-node-detail',
  '04-problems-panel', '05-execution-detail', '06-executions-queue',
  '07-human-pick', '08-human-edit', '09-assets-library',
  '10-model-library', '11-public-form', '12-pipeline-editor',
  '13-pipeline-run-graph', '14-environment-protection',
  '15-publish-diff', '16-provider-credential',
];
const fallbacks = [
  {}, { channel: 'chrome' }, { channel: 'msedge' },
  ...['C:/Program Files/Google/Chrome/Application/chrome.exe',
    'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
    'C:/Program Files/Microsoft/Edge/Application/msedge.exe']
    .filter(existsSync).map(executablePath => ({ executablePath })),
];
let browser;
const failures = [];
for (const choice of fallbacks) {
  try {
    browser = await chromium.launch({ ...choice, headless: true });
    console.log(`Chromium: ${choice.channel ?? choice.executablePath ?? 'bundled'} (headless)`);
    break;
  } catch (error) { failures.push(error.message.split('\n')[0]); }
}
if (!browser) throw new Error(`No installed Chromium available; nothing downloaded.\n${failures.join('\n')}`);
await mkdir(output, { recursive: true });
try {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1,
    colorScheme: 'light', locale: 'zh-CN', reducedMotion: 'reduce',
  });
  await context.route(/^https?:/, route => route.abort());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const summary = [];
  for (const name of pages) {
    const html = await readFile(resolve(root, `${name}.html`), 'utf8');
    if (html.includes('\r')) throw new Error(`${name}: expected LF line endings`);
    if (html.includes('\uFFFD') || !/[\u4e00-\u9fff]/.test(html)) throw new Error(`${name}: invalid Chinese text`);
    await page.goto(pathToFileURL(resolve(root, `${name}.html`)).href, { waitUntil: 'load' });
    await page.evaluate(() => document.fonts.ready);
    const check = await page.evaluate(() => ({
      callouts: document.querySelectorAll('.callout').length,
      legends: document.querySelectorAll('.legend p').length,
      width: document.documentElement.scrollWidth,
      height: document.documentElement.scrollHeight,
      external: [...document.querySelectorAll('[src],link[href]')].some(el => /^https?:/.test(el.src || el.href)),
      clippedNodes: [...document.querySelectorAll('.node')].filter(el => {
        const parent = el.closest('.canvas').getBoundingClientRect();
        const box = el.getBoundingClientRect();
        return box.bottom > parent.bottom + 1 || box.right > parent.right + 1;
      }).length,
      uniqueCallouts: new Set([...document.querySelectorAll('.callout')].map(el => el.textContent)).size,
    }));
    if (!check.callouts || check.callouts !== check.legends) throw new Error(`${name}: callout/legend mismatch`);
    if (check.width > 1440 || check.external) throw new Error(`${name}: overflow or external resource`);
    if (check.clippedNodes || check.uniqueCallouts !== check.callouts) throw new Error(`${name}: clipped nodes or duplicate callouts`);
    const target = resolve(output, `${name}.png`);
    await page.screenshot({ path: target, fullPage: true, animations: 'disabled' });
    const bytes = (await stat(target)).size;
    if (bytes <= 20 * 1024) throw new Error(`${target}: PNG is too small (${bytes} bytes)`);
    summary.push({ file: `${name}.png`, size: `${(bytes / 1024).toFixed(1)} KB`, callouts: check.callouts, dimensions: `1440 × ${check.height}` });
  }
  if (errors.length) throw new Error(errors.join('\n'));
  console.table(summary);
  console.log('PASS: 16 PNGs; all >20 KB; 1440×900 viewport / DPR 1; no horizontal overflow; matching legends.');
  await context.close();
} finally { await browser.close(); }
