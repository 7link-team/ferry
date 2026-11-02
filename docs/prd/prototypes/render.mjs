import { chromium } from 'playwright';
import { existsSync } from 'node:fs';
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

// Run from any directory. No network, installations or browser downloads.
const root = dirname(fileURLToPath(import.meta.url));
const output = resolve(root, '../images');
const layouts = [
  { name: 'Desktop', width: 1440, height: 900, directory: '' },
  { name: 'Pad', width: 768, height: 1024, directory: 'pad' },
  { name: 'Phone', width: 390, height: 844, directory: 'phone' },
];
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
  const summary = [];
  for (const layout of layouts) {
  await mkdir(resolve(output, layout.directory), { recursive: true });
  const context = await browser.newContext({
    viewport: { width: layout.width, height: layout.height }, deviceScaleFactor: 1,
    colorScheme: 'light', locale: 'zh-CN', reducedMotion: 'reduce',
  });
  await context.route(/^https?:/, route => route.abort());
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  for (const name of pages) {
    console.log(`Rendering ${layout.name}/${name}`);
    const html = await readFile(resolve(root, `${name}.html`), 'utf8');
    if (html.includes('\r')) throw new Error(`${name}: expected LF line endings`);
    if (html.includes('\uFFFD') || !/[\u4e00-\u9fff]/.test(html)) throw new Error(`${name}: invalid Chinese text`);
    await page.goto(pathToFileURL(resolve(root, `${name}.html`)).href, { waitUntil: 'load' });
    await page.evaluate(() => document.fonts.ready);
    const check = await page.evaluate(() => ({
      callouts: document.querySelectorAll('.callout').length,
      legends: document.querySelectorAll('.legend p').length,
      width: document.scrollingElement.scrollWidth,
      height: document.documentElement.scrollHeight,
      external: [...document.querySelectorAll('[src],link[href]')].some(el => /^https?:/.test(el.src || el.href)),
      clippedNodes: [...document.querySelectorAll('.node')].filter(el => {
        if (!el.checkVisibility()) return false;
        const parent = el.closest('.graph-surface').getBoundingClientRect();
        const box = el.getBoundingClientRect();
        return box.bottom > parent.bottom + 1 || box.right > parent.right + 1;
      }).length,
      uniqueCallouts: new Set([...document.querySelectorAll('.callout')].map(el => el.textContent)).size,
      invisibleCallouts: [...document.querySelectorAll('.callout')].filter(el => {
        if (!el.checkVisibility()) return true;
        const box = el.getBoundingClientRect();
        return [...(function* () { for (let p = el.parentElement; p; p = p.parentElement) yield p; })()].some(p => {
          if (!/auto|scroll|hidden/.test(getComputedStyle(p).overflowX)) return false;
          const b = p.getBoundingClientRect();
          return box.left < b.left - 1 || box.right > b.right + 1;
        });
      }).map(el => el.textContent),
      scrollRegions: [...document.querySelectorAll('.table-scroll,.canvas')].filter(el => el.checkVisibility() && el.scrollWidth > el.clientWidth).map(el => el.className),
    }));
    if (!check.callouts || check.callouts !== check.legends) throw new Error(`${name}: callout/legend mismatch`);
    if (check.width > layout.width || check.external) {
      const offenders = await page.evaluate(() => [...document.querySelectorAll('body *')].filter(el => el.checkVisibility() && el.getBoundingClientRect().right > innerWidth + 1 && !el.closest('.table-scroll,.canvas')).map(el => ({ tag: el.tagName, class: el.className, right: el.getBoundingClientRect().right })).slice(0,20));
      throw new Error(`${layout.name}/${name}: overflow ${check.width}/${layout.width} or external resource; ${JSON.stringify(offenders)}`);
    }
    if (check.invisibleCallouts.length) throw new Error(`${layout.name}/${name}: invisible callouts ${check.invisibleCallouts}`);
    if (check.clippedNodes || check.uniqueCallouts !== check.callouts) throw new Error(`${name}: clipped nodes or duplicate callouts`);
    // Exercise the actual responsive controls, including keyboard selection and focus return.
    const drawer = page.locator('.drawer-trigger');
    if (await drawer.isVisible()) {
      await drawer.focus();
      await page.keyboard.press('Enter');
      if (!await page.locator('.navigation-dialog').isVisible()) throw new Error(`${name}: drawer did not open`);
      await page.keyboard.press('Escape');
      if (!await drawer.evaluate(el => el === document.activeElement)) throw new Error(`${name}: drawer did not restore focus`);
    }
    const tablist = page.locator('.pane-tabs');
    if (await tablist.isVisible()) {
      const tabs = tablist.locator('[role=tab]');
      const selected = await tabs.evaluateAll(els => els.findIndex(el => el.getAttribute('aria-selected') === 'true'));
      for (let i = 0; i < await tabs.count(); i++) {
        await tabs.nth(i).click();
        const id = await tabs.nth(i).getAttribute('aria-controls');
        if (!await page.locator(`#${id}`).isVisible()) throw new Error(`${name}: unreachable pane ${id}`);
        const hiddenActions = await page.locator(`#${id} button`).evaluateAll(els => els.filter(el => !el.checkVisibility()).map(el => el.textContent));
        if (hiddenActions.length) throw new Error(`${name}: hidden pane actions ${hiddenActions}`);
        if (await page.evaluate(() => document.scrollingElement.scrollWidth > innerWidth)) throw new Error(`${layout.name}/${name}: overflow in pane ${id}`);
      }
      await tabs.nth(selected).focus();
      await page.keyboard.press('ArrowRight');
      const next = (selected + 1) % await tabs.count();
      if (await tabs.nth(next).getAttribute('aria-selected') !== 'true') throw new Error(`${name}: keyboard tabs failed`);
      await tabs.nth(selected).click();
    }
    // Every existing primary button must be reachable in the default layout or one of its panes.
    const inaccessibleActions = await page.locator('main button:not(.pane-tab)').evaluateAll(els => els.filter(el => !el.closest('.pane-inactive') && !el.checkVisibility()).map(el => el.textContent));
    if (inaccessibleActions.length) throw new Error(`${name}: inaccessible actions ${inaccessibleActions}`);
    await page.evaluate(() => { document.activeElement?.blur(); window.scrollTo(0, 0); });
    const target = resolve(output, layout.directory, `${name}.png`);
    await page.screenshot({ path: target, fullPage: true, animations: 'disabled' });
    const bytes = (await stat(target)).size;
    if (bytes <= 15 * 1024) throw new Error(`${target}: PNG is too small (${bytes} bytes)`);
    summary.push({ page: name, layout: layout.name, file: `${layout.directory ? layout.directory + '/' : ''}${name}.png`, bytes, size: `${(bytes / 1024).toFixed(1)} KB`, callouts: check.callouts, viewport: `${layout.width} × ${layout.height}`, dimensions: `${layout.width} × ${check.height}`, scrollWidth: check.width, scrollRegions: check.scrollRegions });
  }
  if (errors.length) throw new Error(errors.join('\n'));
  await context.close();
  }
  // The ADR's compatibility widths are layout regressions, not additional screenshot variants.
  const regression = await browser.newContext({ deviceScaleFactor: 1, reducedMotion: 'reduce' });
  await regression.route(/^https?:/, route => route.abort());
  const probe = await regression.newPage();
  for (const width of [375, 768, 1280, 1920]) {
    await probe.setViewportSize({ width, height: 1024 });
    for (const name of pages) {
      await probe.goto(pathToFileURL(resolve(root, `${name}.html`)).href);
      const nodes = probe.locator('.node');
      if (await nodes.count() > 1 && await nodes.first().isVisible()) {
        await nodes.first().focus();
        await probe.keyboard.press('ArrowDown');
        if (!await nodes.nth(1).evaluate(el => el === document.activeElement)) throw new Error(`ADR ${width}/${name}: node keyboard focus failed`);
        await probe.keyboard.press('Enter');
        const selected = await nodes.nth(1).evaluate(el => el.classList.contains('keyboard-selected'));
        if (!selected) throw new Error(`ADR ${width}/${name}: node Enter failed`);
      }
      const groups = probe.locator('.pane-tabs');
      if (await groups.isVisible()) {
        for (const tab of await groups.locator('button').all()) {
          await tab.click();
          if (await probe.evaluate(() => document.scrollingElement.scrollWidth > innerWidth)) throw new Error(`ADR ${width}/${name}: pane overflow`);
        }
      }
      if (await probe.evaluate(() => document.scrollingElement.scrollWidth > innerWidth)) throw new Error(`ADR ${width}/${name}: page overflow`);
      for (const scroll of await probe.locator('.table-scroll').all()) {
        if (!await scroll.isVisible()) continue;
        const sticky = await scroll.evaluate(el => {
          const first = el.querySelector('tbody td');
          if (!first || el.scrollWidth <= el.clientWidth) return true;
          const before = first.getBoundingClientRect().left;
          el.scrollLeft = el.scrollWidth;
          return Math.abs(first.getBoundingClientRect().left - before) < 1;
        });
        if (!sticky) throw new Error(`ADR ${width}/${name}: first column did not stick`);
      }
    }
    console.log(`PASS ADR width ${width}: 16 pages, all panes, sticky first columns`);
  }
  await regression.close();
  await writeFile(resolve(root, 'render-report.json'), JSON.stringify(summary, null, 2) + '\n', 'utf8');
  const delivery = resolve(root, 'RESPONSIVE.md');
  if (existsSync(delivery)) {
    const text = await readFile(delivery, 'utf8');
    const marker = '<!-- PNG-SIZES -->';
    if (text.includes(marker)) {
      const rows = pages.map(name => `| ${name} | ${layouts.map(layout => summary.find(row => row.page === name && row.layout === layout.name).size).join(' | ')} |`).join('\n');
      await writeFile(delivery, `${text.split(marker)[0]}${marker}\n\n| 页面 | Desktop PNG | Pad PNG | Phone PNG |\n|---|---:|---:|---:|\n${rows}\n`, 'utf8');
    }
  }
  console.table(summary.map(({page,layout,size,dimensions,scrollWidth}) => ({page,layout,size,dimensions,scrollWidth})));
  console.log('PASS: 48 PNGs; all >15 KB; three viewports / DPR 1 / full-page; no page horizontal overflow; all numbered callouts visible; drawer and tab keyboard checks passed.');
} finally { await browser.close(); }
