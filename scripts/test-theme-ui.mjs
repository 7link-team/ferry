import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';

const url = process.env.THEME_UI_URL || 'http://127.0.0.1:5193';
const installedChrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const executablePath = process.env.PLAYWRIGHT_CHROME_PATH || (process.platform === 'win32' && existsSync(installedChrome) ? installedChrome : undefined);
const browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
try {
  const context = await browser.newContext({ locale: 'zh-CN', reducedMotion: 'reduce', viewport: { width: 1366, height: 900 } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/**', route => {
    const path = new URL(route.request().url()).pathname;
    let value = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/me') value = { user: { id: '1', name: 'Theme reviewer', email: 'reviewer@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions: ['hosts.read', 'hosts.create', 'hosts.update', 'hosts.delete', 'host-groups.read'] }] };
    if (path.endsWith('/node-groups')) value = { items: [{ id: 'group-1', name: 'Production group', parent_id: '', position: 0 }], total: 1, page: 1, page_size: 200 };
    if (path.endsWith('/nodes')) value = { items: [{ id: '101', name: 'Theme test host', address: '192.0.2.1', port: 22, username: 'ops', connection: 'ssh', state: 'online', group_id: '', tag_ids: [], tags: [], credential_id: '', jump_host_id: '', description: '', seen_at: Date.now(), version: 1 }], total: 1, page: 1, page_size: 20 };
    if (path === '/api/setup') value = { required: false, has_users: true };
    if (path.endsWith('/preferences')) value = { default_page_size: 20, dictionaries: {} };
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
  });
  await page.addInitScript(() => { if (!localStorage.getItem('ferry.theme')) localStorage.setItem('ferry.theme', JSON.stringify({ palette: 'erp', mode: 'dark' })); });
  await page.goto(`${url}/zh-CN/workspaces/2/nodes`);
  const themeButton = page.getByRole('button', { name: '主题', exact: true });
  await themeButton.waitFor();
  await page.getByRole('button', { name: 'Production group', exact: true }).click();
  assert.equal(await page.locator('html').getAttribute('data-theme'), 'modern');
  assert.equal(await page.locator('html').evaluate(node => node.classList.contains('dark')), true);
  const palettes = [
    { value: 'modern', label: '现代简约' }, { value: 'business', label: '商务渐变' },
    { value: 'eyeCare', label: '护眼模式' }, { value: 'pinkKawaii', label: '粉色可爱' },
    { value: 'neoBrutalism', label: '新粗野主义' }, { value: 'custom', label: '自定义' },
  ];
  mkdirSync('data/maintenance', { recursive: true });
  const source = readFileSync(new URL('../src/shared/theme/themes.css', import.meta.url), 'utf8');
  const raftTokens = JSON.parse(readFileSync(new URL('../src/shared/theme/raft/tokens.json', import.meta.url), 'utf8'));
  const raftColor = value => `oklch(${value.components.join(' ')}${value.alpha !== undefined ? ` / ${value.alpha}` : ''})`;
  const buttonSource = readFileSync(new URL('../src/shared/ui/button.tsx', import.meta.url), 'utf8');
  const buttonBase = /cva\(\s*"([^"]+)"/.exec(buttonSource)[1];
  const buttonVariants = Object.fromEntries([...buttonSource.split('variant: {')[1].split('size: {')[0].matchAll(/(\w+):\s*"([^"]+)"/g)].map(([, key, value]) => [key, value]));
  // Utilities used by native pickers and editor controls must override theme defaults.
  await page.evaluate(() => {
    const fixture = document.createElement('div');
    fixture.id = 'theme-utilities';
    fixture.hidden = true;
    fixture.innerHTML = '<input class="rounded-none border-0 bg-transparent"><input type="color" class="border-0"><h3 class="font-mono">Code</h3><span class="border"></span>';
    document.body.append(fixture);
  });
  const choose = async name => { await themeButton.click(); await page.getByRole('menuitem', { name, exact: false }).click(); };
  for (const palette of palettes) {
    await choose(palette.label);
    if (palette.value === 'custom') await page.getByRole('button', { name: '关闭', exact: true }).click();
    for (const mode of ['light', 'dark']) {
      await choose(mode === 'light' ? '浅色' : '深色');
      const selector = `[data-theme="${palette.value}"]${mode === 'dark' ? '.dark' : ''} {`;
      const block = source.split(selector).slice(1).map(part => part.split('}')[0]).find(part => part.includes('--background:'));
      const isRaft = palette.value === 'neoBrutalism';
      const expected = isRaft ? raftColor(mode === 'dark' ? raftTokens.layer.canvas.$extensions['build.botiverse.raft'].modes.dark : raftTokens.layer.canvas.$value) : /--background:\s*([^;]+);/.exec(block)[1];
      await page.waitForFunction(({ expected, isRaft }) => {
        const probe = document.createElement('span'); document.body.append(probe);
        probe.style.backgroundColor = expected; const color = getComputedStyle(probe).backgroundColor;
        probe.style.backgroundColor = isRaft ? 'var(--raft-layer-canvas)' : 'var(--background)';
        const actual = getComputedStyle(probe).backgroundColor; probe.remove(); return actual === color;
      }, { expected, isRaft });
      const styles = await page.locator('#theme-utilities').evaluate(fixture => [...fixture.children].map(node => {
        const css = getComputedStyle(node);
        return { border: css.borderTopWidth, radius: css.borderRadius, background: css.backgroundColor, font: css.fontFamily };
      }));
      assert.equal(styles[0].border, '0px');
      assert.equal(styles[0].radius, '0px');
      assert.equal(styles[0].background, 'rgba(0, 0, 0, 0)');
      assert.equal(styles[1].border, '0px');
      assert.match(styles[2].font, isRaft ? /Raft Space Mono/ : /Geist Mono Variable/);
      assert.equal(styles[3].border, '1px');
      const triggerStyle = await themeButton.evaluate(node => {
        const css = getComputedStyle(node);
        return { height: css.height, radius: css.borderRadius };
      });
      assert.equal(triggerStyle.height, '32px');
      if (palette.value !== 'custom') assert.equal(triggerStyle.radius, { modern: '11.2px', business: '9.6px', eyeCare: '6.4px', pinkKawaii: '19.2px', neoBrutalism: '8px' }[palette.value], 'reference trigger uses the palette control shape');
      await page.mouse.move(0, 0);
      const shellStyles = await page.evaluate(async () => {
        await Promise.all(document.getAnimations().filter(animation => animation.effect?.getTiming().iterations !== Infinity).map(animation => animation.finished.catch(() => {})));
        const token = (name, property = 'backgroundColor') => {
          const probe = document.createElement('span'); probe.style[property] = `var(${name})`; document.body.append(probe);
          const result = getComputedStyle(probe)[property]; probe.remove(); return result;
        };
        const read = selector => {
          const node = document.querySelector(selector); if (!node) return null;
          const css = getComputedStyle(node); return { background: css.backgroundColor, image: css.backgroundImage, color: css.color };
        };
        return {
          header: read('header'), topActive: read('header nav a[aria-current="page"]'),
          sidebar: read('nav.bg-sidebar'), sideActive: read('nav.bg-sidebar a[aria-current="page"]'),
          groupActive: read('aside .group:has(button[aria-current="page"])'),
          background: token('--background'), foreground: token('--foreground', 'color'), sidebarBackground: token('--sidebar'),
          selected: token('--sidebar-primary'), selectedForeground: token('--sidebar-primary-foreground', 'color'),
          primary: token('--primary'), primaryForeground: token('--primary-foreground', 'color'),
          frame: token('--raft-layer-frame'), frameActive: token('--raft-layer-frame-active'), frameActiveForeground: token('--raft-foreground-on-frame-active', 'color'), raftSelected: token('--raft-selected'), raftInk: token('--raft-sticker-ink', 'color'),
        };
      });
      assert.equal(shellStyles.header.background, isRaft ? shellStyles.frame : shellStyles.background, `${palette.value}/${mode} header uses the reference background surface`);
      assert.equal(shellStyles.sidebar.background, shellStyles.sidebarBackground, `${palette.value}/${mode} sidebar tracks its palette`);
      for (const active of [shellStyles.topActive, shellStyles.sideActive]) {
        assert(active, 'active navigation exists');
        const top = active === shellStyles.topActive;
        assert.equal(active.color, isRaft && top ? shellStyles.frameActiveForeground : shellStyles.selectedForeground, `${palette.value}/${mode} active navigation foreground`);
        if (palette.value === 'business') assert.match(active.image, /linear-gradient/, 'business active navigation keeps the reference gradient');
        else assert.equal(active.background, isRaft && top ? shellStyles.frameActive : shellStyles.selected, `${palette.value}/${mode} selected navigation uses its semantic signal`);
      }
      assert(shellStyles.groupActive, 'actual selected repository/host group exists');
      assert.equal(shellStyles.groupActive.background, isRaft ? shellStyles.raftSelected : shellStyles.primary, 'selected group uses the selected palette color');
      assert.equal(shellStyles.groupActive.color, isRaft ? shellStyles.raftInk : shellStyles.primaryForeground, 'selected group has the matching foreground');
      const dataRow = page.locator('tbody tr[data-selected]').first();
      await dataRow.waitFor();
      const waitForRow = async state => page.waitForFunction(({ row, state, isRaft }) => {
        const probe = document.createElement('span');
        probe.style.backgroundColor = isRaft ? 'var(--background)' : state === 'hover' ? 'color-mix(in oklab, var(--muted) 50%, var(--background))' : state === 'selected' ? 'var(--muted)' : 'var(--background)';
        document.body.append(probe); const expected = getComputedStyle(probe).backgroundColor; probe.remove();
        return [...row.cells].every(cell => getComputedStyle(cell).backgroundColor === expected);
      }, { row: await dataRow.elementHandle(), state, isRaft });
      const rowState = () => dataRow.evaluate(row => {
        const color = value => {
          const probe = document.createElement('span'); probe.style.backgroundColor = value; document.body.append(probe);
          const result = getComputedStyle(probe).backgroundColor; probe.remove(); return result;
        };
        const expected = {
          normal: color('var(--background)'),
          hover: color('color-mix(in oklab, var(--muted) 50%, var(--background))'),
          selected: color('var(--muted)'),
        };
        return { backgrounds: [...row.cells].map(cell => getComputedStyle(cell).backgroundColor), images: [...row.cells].map(cell => getComputedStyle(cell).backgroundImage), expected };
      });
      const normalRow = await rowState();
      assert(normalRow.backgrounds.length > 2 && normalRow.backgrounds.every(background => background === normalRow.expected.normal), `${palette.value}/${mode} sticky and regular cells use one opaque reference surface`);
      await dataRow.hover();
      await waitForRow('hover');
      const hoveredRow = await rowState();
      assert(hoveredRow.backgrounds.every(background => background === (isRaft ? hoveredRow.expected.normal : hoveredRow.expected.hover)), `${palette.value}/${mode} all hovered cells share the reference hover surface: ${JSON.stringify(hoveredRow)}`);
      if (isRaft) assert(hoveredRow.images.every(image => image.includes('linear-gradient')), 'Raft hover lays an ink overlay over opaque paper');
      await page.screenshot({ path: `data/maintenance/theme-row-hover-${palette.value}-${mode}.png` });
      await dataRow.getByRole('checkbox').click();
      await page.waitForFunction(row => row.dataset.selected === 'true', await dataRow.elementHandle());
      await page.mouse.move(0, 0);
      await waitForRow('selected');
      const selectedRow = await rowState();
      assert(selectedRow.backgrounds.every(background => background === (isRaft ? selectedRow.expected.normal : selectedRow.expected.selected)), `${palette.value}/${mode} selected cells share the reference surface: ${JSON.stringify(selectedRow)}`);
      if (isRaft) assert(selectedRow.images.every(image => image.includes('linear-gradient')), 'Raft selected content uses its cyan overlay');
      await page.screenshot({ path: `data/maintenance/theme-row-selected-${palette.value}-${mode}.png` });
      await dataRow.getByRole('checkbox').click();
      await page.mouse.move(0, 0);
      await page.evaluate(({ base, variants }) => {
        const panel = document.createElement('div');
        panel.id = 'theme-buttons';
        panel.style.cssText = 'position:fixed;bottom:80px;right:40px;z-index:40;display:flex;gap:12px;padding:24px;border:1px solid var(--border);background:var(--card)';
        for (const [variant, classes] of Object.entries(variants)) {
          const button = document.createElement('button');
          button.type = variant === 'destructive' ? 'submit' : 'button';
          button.dataset.slot = 'button'; button.dataset.variant = variant;
          button.className = `${base} ${classes} h-9 px-4 py-2`;
          button.textContent = variant; panel.append(button);
        }
        document.body.append(panel);
      }, { base: buttonBase, variants: buttonVariants });
      assert.equal(await page.locator('#theme-buttons button').count(), 6);
      if (palette.value !== 'custom') assert.equal(await page.locator('#theme-buttons button').first().evaluate(node => getComputedStyle(node).borderRadius), { modern: '9999px', business: '10px', eyeCare: '8px', pinkKawaii: '9999px', neoBrutalism: '8px' }[palette.value], 'shared buttons retain their palette-specific radius');
      if (isRaft) {
        await themeButton.click();
        const chosenPalette = page.getByRole('menuitem', { name: palette.label, exact: true });
        const chosenMode = page.getByRole('group', { name: '外观', exact: true }).getByRole('menuitem', { name: mode === 'light' ? '浅色' : '深色', exact: true });
        const chosenStyle = await chosenPalette.evaluate(node => ({ color: getComputedStyle(node).color, background: getComputedStyle(node).backgroundColor, weight: getComputedStyle(node).fontWeight }));
        assert.equal(chosenStyle.color, shellStyles.foreground); assert.equal(chosenStyle.background, 'rgba(0, 0, 0, 0)'); assert.equal(chosenStyle.weight, '700');
        assert.equal(await chosenMode.evaluate(node => getComputedStyle(node).backgroundColor), shellStyles.raftSelected);
        await page.screenshot({ path: `data/maintenance/theme-raft-menu-${mode}.png` });
        await page.keyboard.press('Escape');
        const primaryButton = page.locator('#theme-buttons [data-variant="default"]');
        const before = await primaryButton.boundingBox();
        const resting = await primaryButton.evaluate(node => {
          const css = getComputedStyle(node); return { border: css.borderTopWidth, color: css.color, font: css.fontFamily, transform: css.transform, shadow: css.boxShadow, casing: css.textTransform };
        });
        assert(['1px', '1.5px'].includes(resting.border), 'Raft 1.5px stroke snaps to the device pixel grid'); assert.equal(resting.color, shellStyles.raftInk);
        assert.equal(resting.transform, 'none'); assert.equal(resting.casing, 'none');
        assert.match(resting.font, /Raft Space Grotesk/); assert(!resting.shadow.includes('4px 4px'), 'Raft controls have no hard offset shadow');
        await primaryButton.hover();
        const hovered = await primaryButton.boundingBox();
        assert.equal(hovered.x, before.x); assert.equal(hovered.y, before.y);
        await primaryButton.dispatchEvent('mousedown');
        assert.equal(await primaryButton.evaluate(node => getComputedStyle(node).transform), 'none', 'Raft press never translates a control');
        await primaryButton.dispatchEvent('mouseup'); await page.mouse.move(0, 0);
        const field = page.locator('main input[data-slot="input"]').first();
        assert(['1px', '1.5px'].includes(await field.evaluate(node => getComputedStyle(node).borderTopWidth)), 'Raft field stroke snaps to the device pixel grid');
        assert.equal(await field.evaluate(node => getComputedStyle(node).borderRadius), '8px');
        await field.focus();
        assert.notEqual(await field.evaluate(node => getComputedStyle(node).boxShadow), 'none', 'Raft keyboard focus keeps an ink ring');
        await primaryButton.evaluate(node => { node.disabled = true; });
        const disabled = await primaryButton.evaluate(node => ({ opacity: getComputedStyle(node).opacity, shadow: getComputedStyle(node).boxShadow }));
        assert.equal(disabled.opacity, '1'); assert.equal(disabled.shadow, 'none');
        await primaryButton.evaluate(node => { node.disabled = false; });
        await page.evaluate(async () => { await document.fonts.load('14px "Raft Space Grotesk"'); await document.fonts.load('12px "Raft Space Mono"'); });
        assert(await page.evaluate(() => document.fonts.check('14px "Raft Space Grotesk"') && document.fonts.check('12px "Raft Space Mono"')));
        await page.getByRole('button', { name: '删除', exact: true }).first().click();
        const raftDialog = page.getByRole('dialog');
        await raftDialog.waitFor();
        const dialogStyles = await raftDialog.evaluate(node => {
          const css = getComputedStyle(node); return { radius: css.borderRadius, shadow: css.boxShadow, background: css.backgroundColor, animation: css.animationName };
        });
        assert.equal(dialogStyles.radius, '16px'); assert.notEqual(dialogStyles.shadow, 'none'); assert.equal(dialogStyles.animation, 'none');
        await page.emulateMedia({ reducedMotion: 'no-preference' });
        assert.equal(await page.evaluate(() => {
          const status = document.createElement('span'); status.className = 'motion-safe:animate-pulse'; document.body.append(status);
          const animation = getComputedStyle(status).animationName; status.remove(); return animation;
        }), 'none', 'Raft status stays still even without a reduced-motion preference');
        await page.emulateMedia({ reducedMotion: 'reduce' });
        await page.screenshot({ path: `data/maintenance/theme-raft-dialog-${mode}.png` });
        await raftDialog.getByRole('button', { name: '取消', exact: true }).click();
      }
      const destructiveImage = await page.locator('#theme-buttons [data-variant="destructive"]').evaluate(node => getComputedStyle(node).backgroundImage);
      if (palette.value === 'business' || palette.value === 'pinkKawaii') {
        const primaryImage = await page.locator('#theme-buttons [data-variant="default"]').evaluate(node => getComputedStyle(node).backgroundImage);
        assert.match(primaryImage, /gradient/);
        assert.notEqual(destructiveImage, primaryImage, 'destructive submit must not acquire the primary gradient');
      }
      await page.screenshot({ path: `data/maintenance/theme-buttons-${palette.value}-${mode}.png` });
      await page.locator('#theme-buttons').evaluate(node => node.remove());
      await page.screenshot({ path: `data/maintenance/theme-${palette.value}-${mode}.png` });
    }
  }
  await themeButton.click();
  assert.equal(await page.getByRole('menuitem').filter({ has: page.locator('span[style*="background"]') }).count(), 6);
  const modeGroup = page.getByRole('group', { name: '外观', exact: true });
  assert.equal(await modeGroup.getByRole('menuitem').count(), 3);
  const segments = await modeGroup.getByRole('menuitem').evaluateAll(items => items.map(item => ({ x: item.getBoundingClientRect().x, y: item.getBoundingClientRect().y, width: item.getBoundingClientRect().width })));
  assert(segments.every(item => item.y === segments[0].y), 'appearance modes share one segmented row');
  assert(segments[0].x < segments[1].x && segments[1].x < segments[2].x, 'system/light/dark segments are ordered');
  await page.screenshot({ path: 'data/maintenance/theme-menu.png' });
  assert.equal(await page.getByRole('menuitem', { name: /ERP/i }).count(), 0);
  await page.keyboard.press('Escape');
  // Exercise the shared resource form with each theme, not just token values.
  for (const palette of palettes) {
    await choose(palette.label);
    if (palette.value === 'custom') await page.getByRole('button', { name: '关闭', exact: true }).click();
    for (const mode of ['light', 'dark']) {
      await choose(mode === 'light' ? '浅色' : '深色');
      await page.goto(`${url}/zh-CN/workspaces/2/nodes/new`);
      await page.getByRole('tab', { name: 'SSH', exact: true }).click();
      if (palette.value === 'neoBrutalism') assert.equal(await page.getByRole('tab', { name: 'SSH', exact: true }).evaluate(node => {
        const probe = document.createElement('span'); probe.style.backgroundColor = 'var(--raft-selected)'; document.body.append(probe);
        const expected = getComputedStyle(probe).backgroundColor; probe.remove(); return getComputedStyle(node).backgroundColor === expected;
      }), true, 'Raft form tabs use the yellow selected navigation sticker');
      await page.locator('form').waitFor();
      await page.locator('form input').first().fill('Theme test host');
      await page.screenshot({ path: `data/maintenance/theme-form-${palette.value}-${mode}.png`, fullPage: true });
    }
  }
  await page.goto(`${url}/zh-CN/workspaces/2/nodes`);
  await themeButton.waitFor();
  await page.evaluate(async () => {
    await document.fonts.load('14px "Inter Variable"');
    await document.fonts.load('14px "Geist Mono Variable"');
  });
  assert.equal(await page.evaluate(() => document.fonts.check('14px "Inter Variable"') && document.fonts.check('14px "Geist Mono Variable"')), true);
  await choose('自定义');
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('tab', { name: '浅色', exact: true }).click();
  await dialog.locator('input[type="text"][aria-label="主色"]').fill('#123456');
  await dialog.getByRole('button', { name: '关闭', exact: true }).click();
  await choose('浅色');
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#123456');
  await page.reload(); await themeButton.waitFor();
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#123456');
  await choose('自定义');
  await dialog.getByRole('button', { name: '导入', exact: true }).click();
  await dialog.getByRole('textbox', { name: '将导出的 JSON 粘贴到此处' }).fill('{bad');
  await dialog.getByRole('button', { name: '应用导入' }).click();
  await dialog.getByRole('status').filter({ hasText: '导入失败' }).waitFor();
  await dialog.getByRole('textbox', { name: '将导出的 JSON 粘贴到此处' }).fill(JSON.stringify({ light: { '--primary': '#654321', '--success': '#abcdef' } }));
  await dialog.getByRole('button', { name: '应用导入' }).click();
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#654321');
  await dialog.getByRole('button', { name: '随机配色' }).click();
  assert.notEqual(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '#654321');
  await dialog.getByRole('button', { name: '重置', exact: true }).click();
  assert.equal(await page.evaluate(() => document.documentElement.style.getPropertyValue('--primary')), '');
  await page.setViewportSize({ width: 390, height: 844 });
  const bounds = await dialog.boundingBox();
  assert(bounds.x >= 0 && bounds.x + bounds.width <= 390 && bounds.height <= 844);
  await dialog.evaluate(node => { node.scrollTop = 0; });
  mkdirSync('data/maintenance', { recursive: true });
  await page.screenshot({ path: 'data/maintenance/theme-mobile.png', fullPage: true });
  const translations = JSON.parse(readFileSync(new URL('../src/shared/i18n/locales/theme.json', import.meta.url), 'utf8'));
  await page.setViewportSize({ width: 1366, height: 900 });
  for (const [locale, catalog] of Object.entries(translations)) {
    await page.goto(`${url}/${locale}/workspaces/2/nodes`);
    const localizedTheme = page.getByRole('button', { name: catalog.label, exact: true });
    await localizedTheme.click();
    for (const palette of palettes) await page.getByRole('menuitem', { name: catalog[palette.value], exact: false }).waitFor();
    await page.getByRole('menuitem', { name: catalog.custom, exact: false }).click();
    const localizedDialog = page.getByRole('dialog');
    await localizedDialog.getByRole('heading', { name: catalog.customize, exact: true }).waitFor();
    await localizedDialog.getByLabel(catalog['customPanel.colors.primary'], { exact: true }).first().waitFor();
    assert.equal(await page.locator('html').getAttribute('lang'), locale);
    await localizedDialog.getByRole('button', { name: catalog.close, exact: true }).click();
  }
  assert.deepEqual(errors, []);
  console.log('PASS: 6 palettes x light/dark, actual top/sidebar/group selected colors, header/sidebar surfaces, sticky+regular table normal/hover/selected backgrounds, real forms, six button variants, utility precedence, bundled fonts, ERP migration/removal, custom editing/persistence/import/random/reset, mobile dialog, all six translated dialogs, no page errors');
} finally { await browser.close(); }
