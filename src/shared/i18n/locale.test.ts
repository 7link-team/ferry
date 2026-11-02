import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { currentLocale,initLocale,localeCatalogs,localeHeaders,localizedPath,negotiateLocale,pathLocale,setLocale,stripLocalePrefix,supportedLocales } from "./locale";

beforeEach(() => { localStorage.clear(); history.replaceState({}, "", "/"); });
afterEach(() => { localStorage.clear(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it("negotiates quality weights, aliases, invalid ranges and browser preference order", () => {
  expect(negotiateLocale(undefined)).toBe("en-US");
  expect(negotiateLocale("de-DE, ko;q=0.7, ja-JP;q=0.9")).toBe("ja-JP");
  expect(negotiateLocale("zh-CN;q=0, ms;q=0.4")).toBe("ms-MY");
  expect(negotiateLocale("zh-CN;q=oops, ko;q=1.5, en;q=0.5")).toBe("en-US");
  expect(negotiateLocale(["fr-FR", "zh-Hant-HK", "en-US"])).toBe("zh-TW");
  for (const range of ["zh-HK", "zh-MO", "zh-Hant", "zh-TW"]) expect(negotiateLocale(range)).toBe("zh-TW");
  for (const range of ["zh", "zh-SG", "zh-Hans", "zh-CN"]) expect(negotiateLocale(range)).toBe("zh-CN");
  expect(negotiateLocale("en-GB")).toBe("en-US");
  expect(negotiateLocale("ja;q=0.7, ko;q=0.7")).toBe("ja-JP");
  expect(negotiateLocale("xx-invalid;q=1, *;q=0.1")).toBe("en-US");
});

it("keeps exact locale boundaries and encoded route segments", () => {
  expect(pathLocale("/ZH-cn/workspaces/1/nodes")).toBe("zh-CN");
  expect(pathLocale("/en-USish/workspaces/1/nodes")).toBeUndefined();
  expect(stripLocalePrefix("/ja-JP/workspaces/a%2Fb/nodes")).toBe("/workspaces/a%2Fb/nodes");
  expect(stripLocalePrefix("/ja-JP")).toBe("/");
  expect(localizedPath("/zh-CN/workspaces/1/nodes", "ko-KR")).toBe("/ko-KR/workspaces/1/nodes");
});

it("preserves query, fragment, form history and one history entry on language change", () => {
  history.replaceState({ filePane: "C:/work" }, "", "/zh-CN/workspaces/1/nodes/2/edit?page=3&at=42#setup=local-proof");
  const length = history.length;
  setLocale("ja-JP");
  expect(location.pathname).toBe("/ja-JP/workspaces/1/nodes/2/edit");
  expect(location.search).toBe("?page=3&at=42");
  expect(location.hash).toBe("#setup=local-proof");
  expect(history.state).toEqual({ filePane: "C:/work" });
  expect(history.length).toBe(length);
  expect(document.documentElement.lang).toBe("ja-JP");
  expect(document.documentElement.dir).toBe("ltr");
});

it("lets shared URL locale override saved preference and browser language", () => {
  localStorage.setItem("ui.locale", "ms-MY");
  history.replaceState({}, "", "/ko-KR/account");
  expect(currentLocale()).toBe("ko-KR");
  history.replaceState({}, "", "/account?tab=security#anchor");
  initLocale();
  expect(location.href).toContain("/ms-MY/account?tab=security#anchor");
  expect(localeHeaders({ "accept-language": "ja-JP", "X-Test": "value" })).toEqual({ "x-test": "value", "Accept-Language": "ms-MY" });
});

it("detects browser language and tolerates unavailable preference storage", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("disabled"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("disabled"); });
  vi.stubGlobal("navigator", { languages: ["fr-FR", "ja-JP"] });
  initLocale();
  expect(location.pathname).toBe("/ja-JP/");
  expect(() => setLocale("zh-CN")).not.toThrow();
});

it("ships exactly six complete foundation catalogs with matching interpolation parameters", () => {
  expect(supportedLocales).toEqual(["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]);
  function flatten(value: unknown, prefix = ""): Record<string, string> {
    if (typeof value === "string") return { [prefix]: value };
    return Object.assign({}, ...Object.entries(value as object).map(([key, child]) => flatten(child, `${prefix}${prefix ? "." : ""}${key}`)));
  }
  const english = flatten(localeCatalogs["en-US"]);
  for (const locale of supportedLocales) {
    const entries = flatten(localeCatalogs[locale]);
    expect(Object.keys(entries).sort()).toEqual(Object.keys(english).sort());
    for (const [key, value] of Object.entries(entries)) {
      expect(value.trim(), `${locale}:${key}`).not.toBe("");
      expect(value.match(/\{\{\w+\}\}/g) ?? []).toEqual(english[key]?.match(/\{\{\w+\}\}/g) ?? []);
    }
    if (locale !== "en-US") expect(entries["errors.network"]).not.toBe(english["errors.network"]);
  }
});
