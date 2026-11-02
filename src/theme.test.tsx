import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { applyTheme, initTheme, PALETTES, useTheme } from "./theme";
import { sanitizeCustomThemeVariables } from "./theme-validation";
import { i18n } from "./i18n";
import { setLocale, supportedLocales } from "./locale";
import themeMessages from "./locales/theme.json";
import { ThemeSwitcher } from "./components/ThemeSwitcher";

let root: Root;
let host: HTMLDivElement;
let theme: ReturnType<typeof useTheme>;
let mediaChange: (() => void) | undefined;
let dark = false;
function Probe() { theme = useTheme(); return null; }
beforeEach(() => {
  localStorage.clear();
  dark = false;
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("matchMedia", () => ({ matches: dark, addEventListener: (_: string, listener: () => void) => { mediaChange = listener; }, removeEventListener: () => {} }));
  initTheme();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("migrates saved legacy palettes to modern while preserving the display mode", () => {
  for (const palette of ["erp", "shadcn", "unknown"]) {
    localStorage.setItem("ferry.theme", JSON.stringify({ palette, mode: "dark" }));
    initTheme();
    expect(document.documentElement.dataset.theme).toBe("modern");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.hasAttribute("data-palette")).toBe(false);
  }
  expect(PALETTES.map(p => p.value)).toEqual(["modern", "business", "eyeCare", "pinkKawaii", "neoBrutalism", "custom"]);
});

it("provides complete theme catalogs and reactively translates without changing the selected palette", () => {
  const expected = Object.keys(themeMessages['en-US']).sort();
  for (const locale of supportedLocales) {
    expect(Object.keys(themeMessages[locale]).sort()).toEqual(expected);
    for (const value of Object.values(themeMessages[locale])) expect(value.trim()).not.toBe('');
  }
  act(() => {
    setLocale('en-US');
    root.render(<ThemeSwitcher />);
  });
  expect(host.querySelector('[aria-label="Theme"]')?.textContent).toContain('Modern Minimal');
  act(() => setLocale('ja-JP'));
  expect(host.querySelector('[aria-label="テーマ"]')?.textContent).toContain('モダンミニマル');
  expect(document.documentElement.dataset.theme).toBe('modern');
  expect(i18n.t('importFailed', { ns: 'theme', keySeparator: false })).toContain('JSON');
  act(() => setLocale('zh-CN'));
});
it("persists separate light and dark customization, clears overrides on leaving custom, and restores on reload", () => {
  act(() => root.render(<Probe />));
  act(() => { theme.set({ palette: "custom", mode: "light" }); theme.setCustomVariable("light", "--primary", "#123456"); theme.setCustomVariable("dark", "--primary", "#abcdef"); });
  expect(document.documentElement.style.getPropertyValue("--primary")).toBe("#123456");
  act(() => theme.set({ mode: "dark" }));
  expect(document.documentElement.style.getPropertyValue("--primary")).toBe("#abcdef");
  act(() => theme.set({ palette: "business" }));
  expect(document.documentElement.style.getPropertyValue("--primary")).toBe("");
  act(() => theme.set({ palette: "custom" }));
  initTheme();
  expect(document.documentElement.style.getPropertyValue("--primary")).toBe("#abcdef");
  act(() => theme.resetCustomVariables());
  expect(theme.customLight).toEqual({}); expect(theme.customDark).toEqual({});
});
it("applies system mode changes immediately to the document and hook", () => {
  act(() => root.render(<Probe />));
  act(() => { dark = true; mediaChange?.(); });
  expect(theme.resolved).toBe("dark");
  expect(document.documentElement.style.colorScheme).toBe("dark");
});
it("rejects unrecognized variables and executable CSS but retains semantic status colors", () => {
  expect(sanitizeCustomThemeVariables({ "--primary": "url(https://example.test)", "--unknown": "#123456", "--success": "#123456", "--warning": "#abcdef", "--radius": "9rem" })).toEqual({ "--success": "#123456", "--warning": "#abcdef" });
  localStorage.setItem("ferry.theme", "null"); initTheme();
  applyTheme("modern", "light");
  expect(document.documentElement.dataset.theme).toBe("modern");
});
it("keeps theme changes usable when local storage refuses writes", () => {
  act(() => root.render(<Probe />));
  const spy = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("denied"); });
  try {
    act(() => theme.set({ palette: "eyeCare", mode: "dark" }));
    expect(theme.palette).toBe("eyeCare"); expect(theme.resolved).toBe("dark");
    act(() => theme.set({ mode: "light" }));
    expect(theme.palette).toBe("eyeCare"); expect(theme.resolved).toBe("light");
  } finally { spy.mockRestore(); }
});
