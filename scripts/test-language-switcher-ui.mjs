/** Isolated real-browser coverage of the shared language control and toolbar. */
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { createServer } from "node:http";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { chromium } from "playwright";
const output = resolve("data/maintenance/language-switcher-ui");
await mkdir(output, { recursive: true });
const entry = fileURLToPath(new URL("./__language-entry.tsx", import.meta.url));
const bundle = await build({
  configFile: false,
  logLevel: "warn",
  resolve: {
    alias: { "@": fileURLToPath(new URL("../src", import.meta.url)) },
  },
  plugins: [
    react(),
    tailwindcss(),
    {
      name: "language-fixture",
      resolveId(id) {
        if (id === entry) return id;
      },
      load(id) {
        if (id === entry)
          return `
import React,{useState} from 'react';import {createRoot} from 'react-dom/client';import {useTranslation} from 'react-i18next';import {LanguageSwitcher} from '/src/shared/i18n/LanguageSwitcher';import {ThemeSwitcher} from '/src/shared/theme/ThemeSwitcher';import {initTheme} from '/src/shared/theme/theme';initTheme();import {useRoute} from '/src/app/router';import '/src/style.css';
window.mounts=0;function App(){const{t}=useTranslation();const route=useRoute();const[draft,setDraft]=useState(()=>{window.mounts++;return 'Unsaved multilingual draft';});return <><header className="flex items-center justify-end gap-1 border-b px-4 py-3"><LanguageSwitcher floating={location.pathname.endsWith('/entry')}/><ThemeSwitcher/></header><main className="mx-auto max-w-xl space-y-4 p-6"><h1 className="text-xl font-semibold">Language preferences</h1><label className="grid gap-2 text-sm">Draft<textarea aria-label="Draft" className="min-h-32 rounded-md border bg-background p-3" value={draft} onChange={e=>setDraft(e.target.value)}/></label><p>{t('common.save')}</p><output>{route.workspace}/{route.id}</output></main></>};createRoot(document.getElementById('root')).render(<App/>);`;
      },
    },
  ],
  build: { write: false, rollupOptions: { input: entry } },
});
const assets = bundle.output;
const server = createServer((req, res) => {
  const asset = assets.find(
    (a) =>
      a.fileName === new URL(req.url, "http://localhost").pathname.slice(1),
  );
  if (asset) {
    res.setHeader(
      "Content-Type",
      asset.type === "chunk"
        ? "text/javascript"
        : asset.fileName.endsWith(".css")
          ? "text/css"
          : "application/octet-stream",
    );
    res.end(asset.type === "chunk" ? asset.code : asset.source);
  } else {
    res.setHeader("Content-Type", "text/html");
    res.end(
      `<html><head><meta name="viewport" content="width=device-width, initial-scale=1">${assets
        .filter((a) => a.fileName.endsWith(".css"))
        .map((a) => `<link rel="stylesheet" href="/${a.fileName}">`)
        .join("")}</head><body><div id="root"></div>${assets
        .filter((a) => a.type === "chunk" && a.isEntry)
        .map((a) => `<script type="module" src="/${a.fileName}"></script>`)
        .join("")}</body></html>`,
    );
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const origin = `http://127.0.0.1:${server.address().port}`;
const chrome = process.env.PLAYWRIGHT_CHROME_PATH || "C:/Program Files/Google/Chrome/Application/chrome.exe";
const browser = await chromium.launch({ headless: true, ...(existsSync(chrome) ? { executablePath: chrome } : {}) });
const locales = [
  ["en-US", "English", "Language"],
  ["zh-CN", "简体中文", "语言"],
  ["zh-TW", "繁體中文", "語言"],
  ["ja-JP", "日本語", "言語"],
  ["ko-KR", "한국어", "언어"],
  ["ms-MY", "Bahasa Melayu", "Bahasa"],
];
try {
  const page = await browser.newPage({
    viewport: { width: 1366, height: 800 },
  });
  const errors = [];
  page.on("pageerror", (e) => {
    errors.push(e.message);
    console.error(e.message);
  });
  await page.goto(
    `${origin}/zh-CN/workspaces/123/nodes/456/edit?page=2#details`,
  );
  await page
    .getByLabel("Draft", { exact: true })
    .fill("Keep this edited draft");

  for (const width of [1366, 390, 320]) {
    await page.setViewportSize({ width, height: 800 });
    const toolbarHeights = await page
      .locator('header button[data-size="sm"]')
      .evaluateAll((buttons) =>
        buttons.map((button) => button.getBoundingClientRect().height),
      );
    assert(
      toolbarHeights.length >= 2 &&
        toolbarHeights.every((height) => height === 32),
      "toolbar controls share 32px height",
    );
    for (const [locale, name] of locales) {
      const trigger = page
        .locator('header button[aria-haspopup="menu"]')
        .first();
      await trigger.click();
      await page.getByRole("menuitemradio", { name, exact: true }).click();
      await page.waitForFunction(
        (locale) => document.documentElement.lang === locale,
        locale,
      );
      assert.equal(
        new URL(page.url()).pathname,
        `/${locale}/workspaces/123/nodes/456/edit`,
      );
      assert.equal(
        new URL(page.url()).search + new URL(page.url()).hash,
        "?page=2#details",
      );
      assert.equal(
        await page.getByLabel("Draft", { exact: true }).inputValue(),
        "Keep this edited draft",
      );
      assert.equal(await page.evaluate(() => window.mounts), 1);
      await trigger.focus();
      await page.keyboard.press("ArrowDown");
      const items = page.getByRole("menuitemradio");
      await items.first().waitFor();
      await page.waitForFunction(() => document.activeElement?.getAttribute("role") === "menuitemradio");
      assert.equal(await items.count(), 6);
      assert.equal(await items.locator('span[aria-hidden="true"]').count(), 6, 'each language has a compact locale tag');
      const activeStyle = await page.getByRole("menuitemradio", { name, exact: true }).evaluate(element => {
        const css = getComputedStyle(element);
        return { background: css.backgroundColor, color: css.color, classes: element.className, checks: element.querySelectorAll('svg.lucide-check').length };
      });
      assert.equal(activeStyle.checks, 1, 'selected language shows the reference trailing check');
      assert.match(activeStyle.classes, /data-\[state=checked\]:bg-primary/);
      assert.notEqual(activeStyle.background, 'rgba(0, 0, 0, 0)', 'selected row is visibly filled');
      assert.equal(
        await page
          .getByRole("menuitemradio", { name, exact: true })
          .getAttribute("aria-checked"),
        "true",
      );
      assert.equal(
        (await items.locator("[lang]").count()) +
          (await items.evaluateAll(
            (items) => items.filter((item) => item.hasAttribute("lang")).length,
          )),
        6,
      );
      const menu = await page.getByRole("menu").boundingBox();
      assert(
        menu.x >= 0 && menu.x + menu.width <= width + 1,
        "menu stays inside viewport",
      );
      assert(
        (await page.locator(":focus").getAttribute("role")) === "menuitemradio",
      );
      if (locale === "ms-MY")
        await page.screenshot({
          path: resolve(output, `after-${width}-menu.png`),
        });
      await page.keyboard.press("Escape");
      await page.waitForFunction(
        (element) => document.activeElement === element,
        await trigger.elementHandle(),
      );
      assert(
        await trigger.evaluate((element) => document.activeElement === element),
        "Escape restores trigger focus",
      );
      assert(
        await trigger.evaluate((element) => element.matches(":focus-visible")),
        "keyboard focus remains visible",
      );
      assert(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
        "no horizontal overflow",
      );
    }
  }
  const trigger = page.locator('header button[aria-haspopup="menu"]').first();
  await trigger.focus();
  await page.keyboard.press("Space");
  await page.waitForFunction(() => document.activeElement?.getAttribute("role") === "menuitemradio");
  const keyboardItems = page.getByRole("menuitemradio");
  const expectFocused = async (item) => page.waitForFunction(
    element => document.activeElement === element, await item.elementHandle(),
  );
  await page.keyboard.press("Home");
  await expectFocused(keyboardItems.first());
  await page.keyboard.press("ArrowDown");
  await expectFocused(keyboardItems.nth(1));
  await page.keyboard.press("ArrowUp");
  await expectFocused(keyboardItems.first());
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.documentElement.lang === "en-US");
  await page.waitForFunction(
    (element) => document.activeElement === element,
    await trigger.elementHandle(),
  );
  assert(
    await trigger.evaluate((element) => document.activeElement === element),
  );
  await trigger.click();
  assert.equal(
    await page
      .getByRole("menuitemradio", { name: "English", exact: true })
      .getAttribute("aria-checked"),
    "true",
  );
  await page.keyboard.press("End");
  await expectFocused(page.getByRole("menuitemradio").last());
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.documentElement.lang === "ms-MY");
  await page.getByRole("menu").waitFor({ state: "hidden" });
  await trigger.click();
  await page.getByRole("menu").waitFor();
  // Raw coordinate clicks do not get Playwright's actionability/stability wait.
  // Let Radix finish mounting and registering its outside-pointer listener.
  await page.getByRole("menuitemradio").last().click({ trial: true });
  const outside = await page.getByLabel("Draft", { exact: true }).boundingBox();
  assert(await page.getByRole("menu").evaluate((menu, point) =>
    !menu.contains(document.elementFromPoint(point.x, point.y)),
  { x: outside.x + outside.width - 8, y: outside.y + outside.height - 8 }), "outside click is outside the menu");
  await page.mouse.move(
    outside.x + outside.width - 8,
    outside.y + outside.height - 8,
  );
  await page.mouse.click(
    outside.x + outside.width - 8,
    outside.y + outside.height - 8,
  );
  await page.getByRole("menu").waitFor({ state: "hidden" });
  assert.equal(await trigger.getAttribute("aria-expanded"), "false");
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForFunction(() =>
    document.documentElement.classList.contains("dark"),
  );
  await page.evaluate(async () => {
    await Promise.all(
      document
        .getAnimations()
        .filter(
          (animation) => animation.effect?.getTiming().iterations !== Infinity,
        )
        .map((animation) => animation.finished.catch(() => {})),
    );
  });
  await trigger.click();
  await page.screenshot({
    path: resolve(output, "after-mobile-dark-menu.png"),
  });
  await page.keyboard.press("Escape");
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto(`${origin}/zh-CN/entry`);
  const floating = page.getByRole("button", { name: "语言", exact: true });
  await floating.waitFor();
  const position = await floating.evaluate(
    (element) => getComputedStyle(element.parentElement).position,
  );
  assert.equal(position, "fixed");
  await floating.click();
  assert.equal(await page.getByRole("menuitemradio").count(), 6);
  await page.screenshot({ path: resolve(output, "after-floating-menu.png") });
  assert.deepEqual(errors, []);
  console.log(
    "PASS: six locales × 1366/390/320px, shared toolbar sizing, checked radio state, keyboard open/arrows/Home/End/Enter/Escape and focus, preserved URL/query/hash/draft, light/dark and floating controls, zero runtime errors",
  );
} finally {
  await browser.close();
  await new Promise((r) => server.close(r));
}
