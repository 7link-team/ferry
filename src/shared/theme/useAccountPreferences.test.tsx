import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { useAccountPreferences } from "./useAccountPreferences";
import { initTheme, THEME_PREFERENCE_EVENT } from "./theme";
import { setLocale } from "../i18n/locale";

const { api, errorToast } = vi.hoisted(() => ({ api: vi.fn(), errorToast: vi.fn() }));
vi.mock("../api/api", () => ({ api, errorText: () => "offline" }));
vi.mock("sonner", () => ({ toast: { error: errorToast } }));
let root: Root;
let host: HTMLDivElement;
function Probe({ id }: { id: string | undefined }) { useAccountPreferences(id); return null; }
const theme = { palette: "business", mode: "dark", customLight: {}, customDark: {} };
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
async function mount(id?: string) { await act(async () => { root.render(<Probe id={id} />); await tick(); }); }
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: false }));
  localStorage.clear();
  history.replaceState({ draft: 1 }, "", "/zh-CN/account?tab=profile#name");
  initTheme();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  api.mockReset(); errorToast.mockReset();
  api.mockResolvedValue({ theme, locale: "ms-MY" });
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("restores cloud theme and language without writing them back or losing page state", async () => {
  await mount("1");
  expect(document.documentElement.dataset.theme).toBe("business");
  expect(document.documentElement.classList.contains("dark")).toBe(true);
  expect(location.pathname + location.search + location.hash).toBe("/ms-MY/account?tab=profile#name");
  expect(history.state).toEqual({ draft: 1 });
  expect(api).toHaveBeenCalledTimes(1);
});
it("saves explicit language/theme edits and leaves anonymous settings local", async () => {
  await mount();
  setLocale("en-US");
  expect(api).not.toHaveBeenCalled();
  await mount("1");
  await act(async () => {
    setLocale("ja-JP");
    window.dispatchEvent(new CustomEvent(THEME_PREFERENCE_EVENT, { detail: theme }));
    await tick();
  });
  expect(api.mock.calls.slice(1).map(call => JSON.parse(call[1].body))).toEqual([{ locale: "ja-JP" }, { theme }]);
});
it("ignores stale reads on account switch and preserves edits made during loading", async () => {
  let resolve!: (value: unknown) => void;
  api.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await mount("1");
  await mount("2");
  await act(async () => { resolve({ theme: { ...theme, palette: "pinkKawaii" }, locale: "ja-JP" }); await tick(); });
  expect(document.documentElement.dataset.theme).toBe("business");
  expect(location.pathname).toBe("/ms-MY/account");
  api.mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  await mount("3");
  await act(async () => { setLocale("en-US"); await tick(); resolve({ theme, locale: "ja-JP" }); await tick(); });
  expect(location.pathname).toBe("/en-US/account");
});
it("reports network failure and keeps the local choice", async () => {
  await mount("1");
  api.mockRejectedValueOnce(new Error("offline"));
  await act(async () => { setLocale("en-US"); await tick(); });
  expect(errorToast).toHaveBeenCalledWith("offline");
  expect(location.pathname).toBe("/en-US/account");
});
