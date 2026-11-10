import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n, commonText } from "../shared/i18n/i18n";
import { useDocumentTitle } from "./document-title";
import { navigate, useRoute } from "./router";
import { defaultSiteSettings, SiteSettingsContext, type SiteSettings } from "../shared/site/settings";
import { PlatformFrame, FOOTER_UPDATED } from "./PlatformFooter";

let root: Root;
let host: HTMLDivElement;
let visibility = "visible";
function View() { useDocumentTitle(useRoute()); return null; }
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  visibility = "visible";
  vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility as DocumentVisibilityState);
  history.replaceState({}, "", "/en-US/workspaces/demo/conversations");
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<View />));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it("switches explicitly enabled prefixes without duplicates, retaining the current route while hidden", async () => {
  await act(async () => root.render(<SiteSettingsContext.Provider value={{...defaultSiteSettings,site_title_prefix_enabled:true}}><View /></SiteSettingsContext.Provider>));
  expect(document.title).toBe(`☀️ ${commonText("conversations:nav")} | AI Workspace`);
  visibility = "hidden"; document.dispatchEvent(new Event("visibilitychange"));
  expect(document.title).toBe(`🌙 ${commonText("conversations:nav")} | AI Workspace`);
  await act(async () => navigate({ group: "host", item: "hosts", mode: "list" }, {}));
  expect(document.title).toBe(`🌙 ${commonText("nav.nodes")} | AI Workspace`);
  visibility = "visible";
  for (let i = 0; i < 3; i++) document.dispatchEvent(new Event("visibilitychange"));
  expect(document.title).toBe(`☀️ ${commonText("nav.nodes")} | AI Workspace`);
});

it("updates translated route titles and actions for Chinese, English and Malay", async () => {
  for (const locale of ["zh-CN", "en-US", "ms-MY"]) {
    await act(async () => i18n.changeLanguage(locale));
    await act(async () => navigate({ group: "host", item: "hosts", mode: "new" }, {}));
    expect(document.title).toBe(`${commonText("common.create")} · ${commonText("nav.nodes")} | ${i18n.t("app.title")}`);
  }
});

it("covers public entry, account, platform, and browser history navigation", async () => {
  for (const [path, label] of [
    ["/en-US/login", commonText("auth.signIn")],
    ["/en-US/register", commonText("auth.register")],
    ["/en-US/initialize", commonText("auth.setupTitle")],
    ["/en-US/workspaces", i18n.t("app.workspaces")],
    ["/en-US/account", `${commonText("common.edit")} · ${i18n.t("app.account")}`],
    ["/en-US/platform/users", commonText("resource-domain:platform.users")],
  ]) {
    await act(async () => { history.replaceState({}, "", path!); dispatchEvent(new PopStateEvent("popstate")); });
    expect(document.title).toBe(`${label} | AI Workspace`);
  }
});

it("removes its visibility listener when unmounted", async () => {
  await act(async () => root.render(null));
  document.title = "unmounted";
  visibility = "hidden"; document.dispatchEvent(new Event("visibilitychange"));
  expect(document.title).toBe("unmounted");
});

it("applies custom names, prefixes, empty prefixes and the disable switch immediately", async () => {
  const site: SiteSettings = {...defaultSiteSettings,site_title_prefix_enabled:true,site_name:"Example",site_title_visible_prefix:"🌞",site_title_hidden_prefix:"🌚"};
  const render = async () => act(async () => root.render(<SiteSettingsContext.Provider value={{...site}}><View /></SiteSettingsContext.Provider>));
  await render();
  expect(document.title).toBe(`🌞 ${commonText("conversations:nav")} | Example`);
  visibility="hidden"; document.dispatchEvent(new Event("visibilitychange"));
  expect(document.title).toBe(`🌚 ${commonText("conversations:nav")} | Example`);
  site.site_title_hidden_prefix=""; await render();
  expect(document.title).toBe(`${commonText("conversations:nav")} | Example`);
  site.site_title_prefix_enabled=false; visibility="visible"; await render();
  expect(document.title).toBe(`${commonText("conversations:nav")} | Example`);
  site.site_name=""; await render();
  expect(document.title).toBe(`${commonText("conversations:nav")} | AI Workspace`);
});

it("refreshes public site config after a settings save and keeps it on read failure", async () => {
  let name="Example";
  let fail=false;
  vi.stubGlobal("fetch",vi.fn(async () => {
    if(fail) throw new Error("offline");
    return {ok:true,status:200,json:async()=>({...defaultSiteSettings,content:"",site_name:name})};
  }));
  try {
    await act(async () => root.render(<PlatformFrame><View /></PlatformFrame>));
    expect(document.title).toContain("| Example");
    name="Updated";
    await act(async () => window.dispatchEvent(new Event(FOOTER_UPDATED)));
    expect(document.title).toContain("| Updated");
    fail=true;
    await act(async () => window.dispatchEvent(new Event(FOOTER_UPDATED)));
    expect(document.title).toContain("| Updated");
  } finally {vi.unstubAllGlobals();}
});

it("never flashes default prefixes while a disabled site configuration is still loading", async () => {
  let resolve!: (value: unknown) => void;
  vi.stubGlobal("fetch", vi.fn(() => new Promise(done => { resolve = done; })));
  try {
    await act(async () => root.render(<PlatformFrame><View /></PlatformFrame>));
    expect(document.title).not.toMatch(/☀️|🌙/);
    visibility = "hidden"; document.dispatchEvent(new Event("visibilitychange"));
    expect(document.title).not.toMatch(/☀️|🌙/);
    await act(async () => resolve({ok:true,status:200,json:async()=>({...defaultSiteSettings,site_title_prefix_enabled:false,content:""})}));
    expect(document.title).not.toMatch(/☀️|🌙/);
  } finally { vi.unstubAllGlobals(); }
});
