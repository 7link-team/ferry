/** Real browser coverage of embedded option refresh with native and Radix selectors. */
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

const output = resolve("data/maintenance/select-refresh-ui");
await mkdir(output, { recursive: true });
const entry = fileURLToPath(new URL("./__select-refresh-entry.tsx", import.meta.url));
const bundle = await build({
  configFile: false, logLevel: "warn",
  resolve: { alias: { "@": fileURLToPath(new URL("../src", import.meta.url)) } },
  plugins: [react(), tailwindcss(), {
    name: "select-refresh-fixture",
    resolveId: id => id === entry ? id : undefined,
    load(id) {
      if (id !== entry) return;
      return `
import React,{useState} from 'react';import {createRoot} from 'react-dom/client';
import {AutoForm} from '/src/shared/forms/auto-form';
import {SelectRefresh} from '/src/shared/forms/SelectRefresh';
import {Select,SelectTrigger,SelectValue,SelectContent,SelectItem} from '/src/shared/ui/select';
import {ConnectionRepositoryFields,useConnectionRepositories} from '/src/features/repositories/ConnectionRepositoryFields';
import {RepositorySource} from '/src/features/repositories/RepositorySource';
import {i18n} from '/src/shared/i18n/i18n';import {initTheme} from '/src/shared/theme/theme';import '/src/style.css';
initTheme();i18n.changeLanguage('en-US');window.submits=0;window.optionRefreshes=0;
const navigation={href:()=>'/en-US/workspaces/ws/code-connections/new',navigate:()=>{}};
function App(){const[selection,setSelection]=useState({connection_id:'',name:'',url:'',default_branch:''});const[draft,setDraft]=useState({provider:'git',credential_id:'0'});const data=useConnectionRepositories('ws',selection.connection_id);return <main className="mx-auto max-w-2xl space-y-6 p-6"><AutoForm columns={1} onSubmit={e=>{e.preventDefault();window.submits++;}}>
<ConnectionRepositoryFields navigation={navigation} workspace="ws" data={data} connectionId={selection.connection_id} repositoryUrl={selection.url} repositoryLabel="Repository" onChange={setSelection}/>
<RepositorySource navigation={navigation} workspace="ws" draft={draft} readOnly={false} onChange={patch=>setDraft(value=>({...value,...patch}))} onValidityChange={()=>{}}/>
<SelectRefresh label="Refresh fixture options" onRefresh={()=>window.optionRefreshes++}><Select defaultValue="one"><SelectTrigger aria-label="Fixture selection" className="w-full"><SelectValue/></SelectTrigger><SelectContent><SelectItem value="one">One</SelectItem><SelectItem value="two">Two</SelectItem></SelectContent></Select></SelectRefresh>
<button type="submit">Submit fixture</button></AutoForm></main>};createRoot(document.getElementById('root')).render(<App/>);`;
    },
  }],
  build: { write: false, rollupOptions: { input: entry } },
});
const assets = bundle.output;
const server = createServer((req, res) => {
  const asset = assets.find(value => value.fileName === new URL(req.url, "http://localhost").pathname.slice(1));
  if (asset) {
    res.setHeader("Content-Type", asset.type === "chunk" ? "text/javascript" : asset.fileName.endsWith(".css") ? "text/css" : "application/octet-stream");
    res.end(asset.type === "chunk" ? asset.code : asset.source);
  } else {
    res.setHeader("Content-Type", "text/html");
    res.end(`<html><head><meta name="viewport" content="width=device-width, initial-scale=1">${assets.filter(value => value.fileName.endsWith(".css")).map(value => `<link rel="stylesheet" href="/${value.fileName}">`).join("")}</head><body><div id="root"></div>${assets.filter(value => value.type === "chunk" && value.isEntry).map(value => `<script type="module" src="/${value.fileName}"></script>`).join("")}</body></html>`);
  }
});
await new Promise(done => server.listen(0, "127.0.0.1", done));
const chrome = process.env.PLAYWRIGHT_CHROME_PATH || "C:/Program Files/Google/Chrome/Application/chrome.exe";
const browser = await chromium.launch({ headless: true, ...(existsSync(chrome) ? { executablePath: chrome } : {}) });
try {
  const page = await browser.newPage({ viewport: { width: 1000, height: 900 } });
  const errors = [];
  page.on("pageerror", cause => errors.push(cause.message));
  let connectionReads = 0, credentialReads = 0, holdConnection = false, releaseConnection;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url());
    let data;
    if (url.pathname.endsWith("/credentials")) {
      credentialReads++;
      data = { items: [{ id: "c1", name: "Team credential", kind: "token" }], total: 1, page: 1, page_size: 200 };
    } else if (url.pathname.endsWith("/repositories")) {
      data = { items: [{ name: "project", full_name: "team/project", url: "https://git.example.invalid/team/project.git", default_branch: "main" }], has_more: false };
    } else {
      connectionReads++;
      if (holdConnection) await new Promise(done => { releaseConnection = done; });
      data = { items: [{ id: "11", name: "Team platform", provider: "github", state: "connected" }], total: 1, page: 1, page_size: 200 };
    }
    await route.fulfill({ json: data });
  });
  await page.goto(`http://127.0.0.1:${server.address().port}/en-US/workspaces/ws/code-insights/new`);
  const connection = page.getByRole("combobox", { name: "Repository connection", exact: true });
  await page.waitForFunction(() => document.querySelector('select[aria-label="Repository connection"]')?.options.length === 2);
  await connection.click(); await connection.press("Home"); await connection.press("ArrowDown"); await connection.press("Enter");
  assert.equal(await connection.inputValue(), "11", "actual native mouse/keyboard selection is retained");
  const embedded = page.getByRole("button", { name: "Refresh connections", exact: true });
  await page.waitForFunction(() => !document.querySelector('button[aria-label="Refresh connections"]').disabled);
  assert.equal(await embedded.textContent(), "", "refresh icon has no redundant visible text");
  const frame = connection.locator("..");
  assert.equal(await frame.getAttribute("data-slot"), "select-refresh");
  const selectBox = await connection.boundingBox(), iconBox = await embedded.boundingBox();
  assert(iconBox.x >= selectBox.x && iconBox.x + iconBox.width < selectBox.x + selectBox.width / 2 && iconBox.y >= selectBox.y && iconBox.y + iconBox.height <= selectBox.y + selectBox.height, "refresh is embedded inside the selector's left edge");
  assert.equal(await connection.evaluate(node => getComputedStyle(node).paddingLeft), "40px", "selected text reserves space for the icon");
  holdConnection = true;
  await embedded.click();
  await page.waitForFunction(() => document.querySelector('button[aria-label="Refresh connections"]').disabled);
  assert.equal(await embedded.getAttribute("aria-busy"), "true");
  assert.equal(await embedded.locator("svg.animate-spin").count(), 1);
  while (!releaseConnection) await new Promise(done => setTimeout(done, 10));
  holdConnection = false; releaseConnection();
  await page.waitForFunction(() => !document.querySelector('button[aria-label="Refresh connections"]').disabled);
  assert.equal(await connection.inputValue(), "11", "refresh preserves the selected connection");
  assert(connectionReads >= 2);
  const credential = page.getByRole("combobox", { name: "Credential", exact: true });
  await credential.click(); await credential.press("Home"); await credential.press("ArrowDown"); await credential.press("Enter");
  assert.equal(await credential.inputValue(), "c1");
  const beforeCredentials = credentialReads;
  const refreshCredentials = page.getByRole("button", { name: "Refresh credentials", exact: true });
  await refreshCredentials.focus(); await refreshCredentials.press("Space");
  await page.waitForFunction(() => !document.querySelector('button[aria-label="Refresh credentials"]').disabled);
  assert(credentialReads > beforeCredentials, "keyboard refresh reloads credential options");
  assert.equal(await credential.inputValue(), "c1", "credential refresh retains the selected credential");
  const refreshRadix = page.getByRole("button", { name: "Refresh fixture options", exact: true });
  assert.equal(await page.getByRole("combobox", { name: "Fixture selection", exact: true }).evaluate(node => getComputedStyle(node).paddingLeft), "40px", "Radix selected text reserves space for its embedded icon");
  await refreshRadix.click(); await refreshRadix.focus(); await refreshRadix.press("Enter");
  assert.equal(await page.evaluate(() => window.optionRefreshes), 2);
  assert.equal(await page.getByRole("listbox").count(), 0, "refresh does not open the adjacent Radix dropdown");
  assert.equal(await page.evaluate(() => window.submits), 0, "embedded refresh never submits the enclosing form");
  for (const width of [1000, 390, 320]) {
    await page.setViewportSize({ width, height: 1000 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "no horizontal overflow");
    await page.screenshot({ path: resolve(output, `refresh-${width}.png`), fullPage: true });
  }
  assert.deepEqual(errors, []);
  console.log("PASS: embedded native/Radix refresh icons, native connection selection, mouse/keyboard refresh, loading state, preserved connection/credential, no dropdown activation or form submit, 1000/390/320px layouts");
} finally {
  await browser.close();
  await new Promise(done => server.close(done));
}
