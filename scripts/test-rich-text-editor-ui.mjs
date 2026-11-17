/**
 * Rich-text editor look and fit in every palette, light and dark.
 * Run against a vite preview (default http://127.0.0.1:5193); API calls are fixtures.
 * Screenshots go to data/maintenance/rich-text/ (git-ignored).
 */
import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync } from 'node:fs';

const origin = process.env.RICH_TEXT_UI_URL || 'http://127.0.0.1:5193';
const installedChrome = 'C:/Program Files/Google/Chrome/Application/chrome.exe';
const executablePath = process.env.PLAYWRIGHT_CHROME_PATH || (process.platform === 'win32' && existsSync(installedChrome) ? installedChrome : undefined);
const out = 'data/maintenance/rich-text';
mkdirSync(out, { recursive: true });

// Formatting typical of content written with the previous editor.
const html = '<h2>Deployment report</h2>'
  + '<p>Run <span style="color: rgb(225, 60, 57);"><strong>failed</strong></span> for {{ repo.name }} on '
  + '<span style="background-color: rgb(255, 236, 61); font-size: 14px;">{{ run.branch }}</span>.</p>'
  + '<table><tbody><tr><th>Step</th><th>Status</th></tr><tr><td>Build</td><td><span style="color: rgb(45, 194, 107);">ok</span></td></tr><tr><td>Deploy</td><td>{{ deploy.status }}</td></tr></tbody></table>'
  + '<hr><p>Contact <a href="https://example.invalid/support" target="_blank">support</a>.</p>';
const tableMenu = '表格操作：行、列、删除';
const palettes = ['modern', 'business', 'eyeCare', 'neoBrutalism', 'custom'];
const modes = ['light', 'dark'];

const browser = await chromium.launch({ headless: true, ...(executablePath ? { executablePath } : {}) });
const failures = [];
try {
  const context = await browser.newContext({ locale: 'zh-CN', reducedMotion: 'reduce', viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  let theme = { palette: 'modern', mode: 'light' };
  const saved = [];
  const row = () => ({ id: '501', key: 'mail_footer', name: 'Mail footer', description: '', category: 'general', builtin: false,
    value_type: 'richtext', value_json: JSON.stringify(html), value: html, version: 1, schema_json: '{}' });
  await page.route('**/api/**', async route => {
    const request = route.request(); const path = new URL(request.url()).pathname;
    let value = { items: [], total: 0, page: 1, page_size: 20 };
    if (path === '/api/me') value = { user: { id: '1', name: 'Reviewer', email: 'reviewer@example.invalid', is_superadmin: false }, workspaces: [{ id: '2', name: 'Test workspace', slug: 'ops', role_name: 'Owner', permissions: ['settings.read', 'settings.update'] }] };
    if (path === '/api/me/preferences') value = { theme, locale: null };
    else if (path.endsWith('/preferences')) value = { default_page_size: 20, dictionaries: {} };
    if (path === '/api/setup') value = { required: false, has_users: true };
    if (path === '/api/workspaces/2/settings/501') {
      if (request.method() === 'PATCH') { saved.push(request.postDataJSON()); value = { id: '501', version: 2 }; } else value = row();
    }
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
  });
  await page.addInitScript(() => {
    if (window !== window.top) return; // sandboxed previews have no storage
    const wanted = JSON.parse(sessionStorage.getItem('wanted-theme') || 'null');
    if (wanted) localStorage.setItem('ferry.theme', JSON.stringify(wanted));
  });

  const open = async (palette, mode) => {
    theme = { palette, mode };
    await page.goto(`${origin}/zh-CN/workspaces/2/settings/501/edit`, { waitUntil: 'domcontentloaded' });
    await page.evaluate(wanted => sessionStorage.setItem('wanted-theme', JSON.stringify(wanted)), theme);
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.locator('[data-slate-editor] table').waitFor();
    await page.waitForFunction(([p, m]) => document.documentElement.getAttribute('data-theme') === p && document.documentElement.classList.contains('dark') === (m === 'dark'), [palette, mode]);
  };

  /** Toolbar controls whose text/icon colour equals what is behind them. */
  const unreadable = () => page.evaluate(() => {
    const opaque = element => {
      for (let node = element; node; node = node.parentElement) {
        const color = getComputedStyle(node).backgroundColor;
        if (color && color !== 'transparent' && !/rgba\(.*,\s*0\)$/.test(color)) return color;
      }
      return getComputedStyle(document.body).backgroundColor;
    };
    const toolbar = document.querySelector('[role="toolbar"]');
    return [...toolbar.querySelectorAll('button')].map(button => {
      const icon = button.querySelector('svg');
      const foreground = getComputedStyle(icon ?? button).color;
      return { label: button.getAttribute('aria-label') || button.textContent, foreground, background: opaque(button), disabled: button.disabled };
    }).filter(control => control.foreground === control.background || /rgba\(.*,\s*0\)$/.test(control.foreground));
  });

  for (const palette of palettes) {
    for (const mode of modes) {
      const name = `${palette}-${mode}`;
      console.log(`Checking ${name}`);
      await open(palette, mode);
      const box = page.getByRole('textbox', { name: '富文本值', exact: true });
      assert.equal(await page.locator('[role="alert"]').filter({ hasText: 'HTML' }).count(), 0, `${name}: content opens visually`);
      assert.equal(await page.locator('[data-placeholder-chip]').count(), 3, `${name}: variables are chips`);
      assert.equal(await page.locator('[data-slate-editor] hr').count(), 1, `${name}: horizontal rule`);
      const colored = await page.locator('[data-slate-editor] [style*="color"]').first().evaluate(node => getComputedStyle(node).color);
      assert.equal(colored, 'rgb(225, 60, 57)', `${name}: stored colour is shown as-is`);
      // A highlight without its own text colour gets readable text in every theme (display only).
      const chip = await page.locator('[data-placeholder-chip="{{ run.branch }}"]').evaluate(node => ({ color: getComputedStyle(node).color, background: getComputedStyle(node).backgroundColor }));
      assert.deepEqual(chip, { color: 'rgb(17, 17, 17)', background: 'rgb(255, 236, 61)' }, `${name}: highlighted chip is readable`);

      if (palette === 'modern' && mode === 'light') {
        // Format by hand: colour a word from the palette, then add a table row through the table menu.
        await box.click();
        await page.locator('[data-slate-editor] p').last().click();
        await page.keyboard.press('End');
        await page.keyboard.type(' Thanks');
        await page.keyboard.press('Shift+ControlOrMeta+ArrowLeft');
        await page.getByRole('button', { name: '文字颜色', exact: true }).click();
        await page.getByRole('button', { name: '#1f6feb', exact: true }).click();
        await page.waitForFunction(() => [...document.querySelectorAll('[data-slate-editor] [style]')].some(node => getComputedStyle(node).color === 'rgb(31, 111, 235)' && node.textContent.includes('Thanks')));
        await page.locator('[data-slate-editor] td', { hasText: 'Build' }).click();
        await page.getByRole('button', { name: tableMenu, exact: true }).click();
        await page.getByRole('menuitem', { name: '在下方插入行', exact: true }).click();
        await page.waitForFunction(() => document.querySelectorAll('[data-slate-editor] tr').length === 4);
        await page.getByRole('button', { name: '保存', exact: true }).click();
        await page.waitForTimeout(500);
        assert(saved.length > 0, 'saved');
        const stored = JSON.parse(saved.at(-1).value_json);
        assert.match(stored, /<span style="color: #1f6feb;">Thanks<\/span>/, 'colour saves as an inline style');
        assert.equal((stored.match(/<tr>/g) || []).length, 4, 'new row is saved');
        assert.match(stored, /\{\{ run\.branch \}\}/, 'variables save as written');
        await open(palette, mode);
      }

      // Put the cursor in the table so the table menu is part of the toolbar check.
      await page.locator('[data-slate-editor] td', { hasText: 'Build' }).click();
      await page.getByRole('button', { name: tableMenu, exact: true }).waitFor();
      const bad = await unreadable();
      if (bad.length) failures.push(`${name}: unreadable toolbar controls ${JSON.stringify(bad)}`);
      const editorFrame = page.locator('[role="toolbar"]').locator('..');
      await editorFrame.screenshot({ path: `${out}/${name}-desktop.png` });

      // Colour popover in this theme.
      await page.getByRole('button', { name: '背景颜色', exact: true }).click();
      await page.getByRole('textbox', { name: '自定义颜色（十六进制）', exact: true }).waitFor();
      await page.screenshot({ path: `${out}/${name}-colors.png` });
      await page.keyboard.press('Escape');

      await page.setViewportSize({ width: 390, height: 844 });
      await page.waitForTimeout(200);
      const fit = await page.evaluate(() => {
        const toolbar = document.querySelector('[role="toolbar"]');
        const editor = document.querySelector('[data-slate-editor]');
        return {
          page: document.documentElement.scrollWidth - document.documentElement.clientWidth,
          toolbar: toolbar.scrollWidth - toolbar.clientWidth,
          toolbarRight: toolbar.getBoundingClientRect().right - window.innerWidth,
          editorRight: editor.getBoundingClientRect().right - window.innerWidth,
        };
      });
      if (fit.page > 0 || fit.toolbar > 0 || fit.toolbarRight > 0.5 || fit.editorRight > 0.5) failures.push(`${name}: horizontal overflow at 390px ${JSON.stringify(fit)}`);
      await page.locator('[role="toolbar"]').scrollIntoViewIfNeeded();
      await editorFrame.screenshot({ path: `${out}/${name}-mobile.png` });
      // Scrolled to the end, the editor's last line and the rest of the form sit above the sticky 取消/保存 bar.
      const end = await page.evaluate(() => {
        for (const element of document.querySelectorAll('*')) if (element.scrollHeight > element.clientHeight + 1 && /(auto|scroll)/.test(getComputedStyle(element).overflowY)) element.scrollTop = element.scrollHeight;
        const actions = document.querySelector('[data-form-actions-placement="footer"]');
        const content = [...actions.closest('form').children].filter(child => child !== actions && !child.contains(actions));
        return {
          actionsTop: actions.getBoundingClientRect().top,
          actionsBottom: actions.getBoundingClientRect().bottom,
          lastLine: [...document.querySelectorAll('[data-slate-editor] > *')].at(-1).getBoundingClientRect().bottom,
          editor: document.querySelector('[role="toolbar"]').parentElement.getBoundingClientRect().bottom,
          content: Math.max(...content.map(child => child.getBoundingClientRect().bottom)),
          viewport: innerHeight,
        };
      });
      if (end.lastLine > end.actionsTop || end.editor > end.actionsTop || end.content > end.actionsTop || end.actionsBottom > end.viewport) failures.push(`${name}: form end hidden behind the action bar at 390px ${JSON.stringify(end)}`);
      await page.screenshot({ path: `${out}/${name}-mobile-end.png` });
      await page.setViewportSize({ width: 1280, height: 900 });
    }
  }
  assert.deepEqual(errors, [], 'no browser errors');
  assert.deepEqual(failures, [], failures.join('\n'));
  console.log(`PASS rich-text editor UI: ${palettes.length * modes.length} themes; screenshots in ${out}/`);
} finally {
  await browser.close();
}
