import en from "./locales/en-US.json";
import zh from "./locales/zh-CN.json";
import zhTW from "./locales/zh-TW.json";
import ja from "./locales/ja-JP.json";
import ko from "./locales/ko-KR.json";
import ms from "./locales/ms-MY.json";


/** Add a locale only when its real catalog is available. */
export const localeCatalogs = { "en-US": en, "zh-CN": zh, "zh-TW": zhTW, "ja-JP": ja, "ko-KR": ko, "ms-MY": ms };
export type Locale = keyof typeof localeCatalogs;
export const supportedLocales = Object.keys(localeCatalogs) as Locale[];
export const localeNames: Record<Locale, string> = {"en-US": "English", "zh-CN": "简体中文", "zh-TW": "繁體中文", "ja-JP": "日本語", "ko-KR": "한국어", "ms-MY": "Bahasa Melayu"};
export const DEFAULT_LOCALE: Locale = "en-US";
const preferenceKey = "ui.locale";
export const LOCALE_EVENT = "ui:locale-change";

/** RFC language ranges, ordered by quality; unsupported ranges never hide later matches. */
export function negotiateLocale(header: string | readonly string[] | undefined, supported: readonly string[] = supportedLocales): string {
  const entries = (typeof header === "string" ? header.split(",") : header ?? [])
    .map((entry, index) => {
      const [range = "", ...parameters] = entry.trim().split(";");
      const quality = parameters.find(parameter => parameter.trim().toLowerCase().startsWith("q="));
      const raw = quality?.trim().slice(2);
      const q = raw === undefined ? 1 : /^(?:0(?:\.\d{0,3})?|1(?:\.0{0,3})?)$/.test(raw) ? Number(raw) : 0;
      return { range: range.trim().toLowerCase(), q, index };
    }).filter(entry => entry.q > 0).sort((a, b) => b.q - a.q || a.index - b.index);
  const exact = (tag: string) => supported.find(locale => locale.toLowerCase() === tag);
  for (const { range } of entries) {
    if (range === "*") return exact("en-us") ?? supported[0] ?? DEFAULT_LOCALE;
    if (!/^[a-z]{2,3}(?:-[a-z0-9]{2,8})*$/.test(range)) continue;
    const match = exact(range);
    if (match) return match;
    const language = range.split("-")[0]!;
    const preferred = language === "zh"
      ? /(?:^|-)(hant|tw|hk|mo)(?:-|$)/.test(range) ? "zh-tw" : "zh-cn"
      : ({ en: "en-us", ja: "ja-jp", ko: "ko-kr", ms: "ms-my", pt: "pt-br" } as Record<string, string>)[language];
    const alias = preferred ? exact(preferred) : undefined;
    if (alias) return alias;
    const base = supported.find(locale => locale.toLowerCase().split("-")[0] === language);
    if (base) return base;
  }
  return exact("en-us") ?? supported[0] ?? DEFAULT_LOCALE;
}

export function pathLocale(pathname: string): Locale | undefined {
  const first = pathname.split("/")[1];
  return supportedLocales.find(locale => locale.toLowerCase() === first?.toLowerCase());
}

export function stripLocalePrefix(pathname: string): string {
  return pathLocale(pathname) ? pathname.slice(pathname.indexOf("/", 1) < 0 ? pathname.length : pathname.indexOf("/", 1)) || "/" : pathname;
}

export function localizedPath(pathname: string, locale: Locale): string {
  const path = stripLocalePrefix(pathname);
  return `/${locale}${path.startsWith("/") ? path : `/${path}`}`;
}

function savedLocale(): Locale | undefined {
  try {
    const saved = localStorage.getItem(preferenceKey);
    return supportedLocales.find(locale => locale === saved);
  } catch { return undefined; }
}

export function currentLocale(): Locale {
  return pathLocale(location.pathname) ?? savedLocale() ?? negotiateLocale(navigator.languages) as Locale;
}

export function localeHeaders(headers?: HeadersInit): Record<string, string> {
  const result = Object.fromEntries(new Headers(headers).entries());
  // The URL language wins over browser defaults and stale caller headers.
  for (const key of Object.keys(result)) if (key.toLowerCase() === "accept-language") delete result[key];
  return { ...result, "Accept-Language": currentLocale() };
}

/** No reload or route remount: drafts, query state, fragments and history state survive. */
export function setLocale(locale: Locale, persist = true): void {
  if (!supportedLocales.includes(locale)) return;
  if (persist) try { localStorage.setItem(preferenceKey, locale); } catch { /* Storage is optional. */ }
  const next = localizedPath(location.pathname, locale) + location.search + location.hash;
  history.replaceState(history.state, "", next);
  document.documentElement.lang = locale;
  document.documentElement.dir = locale.startsWith("ar-") ? "rtl" : "ltr";
  window.dispatchEvent(new Event(LOCALE_EVENT));
}

export function initLocale(): void {
  setLocale(currentLocale(), false);
}
