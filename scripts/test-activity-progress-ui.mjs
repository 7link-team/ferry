/** Production-browser acceptance: real app requests with delayed API fixtures. */
import assert from 'node:assert/strict';
import { preview } from 'vite';
import { chromium } from 'playwright';

const server = await preview({ preview: { host: '127.0.0.1', port: 0 } });
const origin = process.env.FERRY_UI_ORIGIN || `http://127.0.0.1:${server.httpServer.address().port}`;
let browser;
try {
  browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROME_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROME_PATH } : {}) });
  for (const width of [1280, 375]) {
    const context = await browser.newContext({ viewport: { width, height: 800 } });
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => {
      window.routePaints = [];
      new MutationObserver(() => {
        const layer = document.querySelector('[data-kind="route"][data-done="false"]');
        if (layer) window.routePaints.push(getComputedStyle(layer).opacity);
      }).observe(document, { childList: true, subtree: true });
    });
    const requests = new Map();
    await page.route('**/api/**', async route => {
      const path = new URL(route.request().url()).pathname;
      if ((path === '/api/me' || path === '/api/setup') && !requests.has(path)) requests.set(path, route);
      else if (path === '/api/setup') await route.fulfill({ json: { required: false } });
      else await route.fulfill({ json: {} });
    });
    await page.goto(`${origin}/en-US/workspaces/1/nodes`);
    await page.waitForFunction(() => document.querySelectorAll('[data-kind="request"][data-done="false"]').length === 2);
    const bar = page.locator('.activity-progress');
    assert.equal((await bar.boundingBox()).height, 1);
    assert.equal(await bar.evaluate(el => getComputedStyle(el).pointerEvents), 'none');
    assert.ok((await page.evaluate(() => window.routePaints)).includes('1'), 'route uses full opacity');
    await page.waitForFunction(() => !document.querySelector('[data-kind="route"]'));
    const layers = page.locator('[data-kind="request"][data-done="false"]');
    for (const theme of ['modern', 'business', 'eyeCare', 'pinkKawaii', 'neoBrutalism', 'custom']) {
      for (const dark of [false, true]) {
      await page.evaluate(({ theme, dark }) => { document.documentElement.dataset.theme = theme; document.documentElement.classList.toggle('dark', dark); }, { theme, dark });
      const colors = await layers.evaluateAll(elements => elements.map(element => {
        const style = getComputedStyle(element);
        return { color: style.backgroundColor, opacity: style.opacity, duration: style.animationDuration };
      }));
      assert.equal(colors.length, 2);
      assert.ok(colors.every(color => color.opacity === '0.62' && color.color !== 'rgba(0, 0, 0, 0)'), JSON.stringify({ theme, colors }));
      assert.notEqual(colors[0].color, colors[1].color, 'endpoints get distinct hues');
      assert.notEqual(colors[0].duration, colors[1].duration, 'requests animate independently');
      }
    }
    await page.emulateMedia({ reducedMotion: 'reduce' });
    assert.equal(await layers.first().evaluate(el => getComputedStyle(el).animationName), 'none');
    await requests.get('/api/setup').fulfill({ json: { required: false } });
    await page.waitForFunction(() => document.querySelectorAll('[data-kind="request"]').length === 1);
    await requests.get('/api/me').fulfill({ status: 401, json: { error: 'Sign in' } });
    await bar.waitFor({ state: 'detached' });
    assert.deepEqual(errors, []);
    await context.close();
    console.log(`PASS activity progress ${width}px: 1px, opaque route, translucent independent requests, themes, reduced motion, completion`);
  }
} finally {
  await browser?.close();
  await new Promise(resolve => server.httpServer.close(resolve));
}
