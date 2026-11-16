// Conversation smoothness check (Playwright). Not part of CI.
//
//   node scripts/chat-perf.mjs <conversation URL> [storageState.json]
//
// Loads a conversation, times opening the first execution details, then
// records main-thread long tasks (>50 ms) during 10 s of idle polling.
// Read-only: it never types or sends.
import { chromium } from "playwright";

const [url, storageState] = process.argv.slice(2);
if (!url) {
  console.error("usage: node scripts/chat-perf.mjs <conversation URL> [storageState.json]");
  process.exit(2);
}
const browser = await chromium.launch();
try {
  const context = await browser.newContext({ ...(storageState ? { storageState } : {}), viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  await page.addInitScript(() => {
    window.__longTasks = [];
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) window.__longTasks.push(Math.round(entry.duration));
    }).observe({ type: "longtask", buffered: true });
  });
  const started = Date.now();
  await page.goto(url);
  await page.locator("textarea").first().waitFor();
  await page.waitForLoadState("networkidle").catch(() => {});
  const loaded = Date.now() - started;
  const details = page.locator("details[data-execution-details] > summary").first();
  let expand = null;
  if (await details.count()) {
    expand = await page.evaluate(async () => {
      const summary = document.querySelector("details[data-execution-details] > summary");
      const t0 = performance.now();
      summary.click();
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      return Math.round(performance.now() - t0);
    });
  }
  await page.evaluate(() => { window.__longTasks = []; });
  await page.waitForTimeout(10_000);
  const idle = await page.evaluate(() => window.__longTasks);
  console.log(JSON.stringify({
    load_ms: loaded,
    expand_details_ms: expand,
    idle_long_tasks: idle.length,
    idle_long_task_ms_total: idle.reduce((sum, value) => sum + value, 0),
    idle_long_task_ms_max: idle.length ? Math.max(...idle) : 0,
  }, null, 2));
} finally {
  await browser.close();
}
