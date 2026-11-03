import i18n from "i18next";
import { initReactI18next,useTranslation } from "react-i18next";
import { currentLocale,DEFAULT_LOCALE,LOCALE_EVENT,localeCatalogs,supportedLocales } from "./locale";
import reportsReviewsMessages from "./locales/reports-reviews-domain";
import commonMessages from "./locales/common-domain.json";
import executionMessages from "./locales/execution-domain.json";
import hostRepositoryMessages from "./locales/host-repository-domain.json";
import insightMessages from "./locales/insight-domain.json";
import insightZh from "./locales/insight-zh.json";
import resourceMessages from "./locales/resource-domain.json";
import themeMessages from "./locales/theme.json";
import typedMessages from "./locales/typed-domain.json";

// Column order is explicit so adding/reordering supported locales cannot change translations.
const typedLocales = ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"];
const insightLocales = ["en-US", "zh-TW", "ja-JP", "ko-KR", "ms-MY"];
const insightResources = Object.fromEntries(typedLocales.map(locale => [locale, Object.fromEntries(Object.entries(insightMessages).map(([key, values]) => [key, locale === "zh-CN" ? insightZh[key as keyof typeof insightZh] ?? key : values[insightLocales.indexOf(locale)]!]))]));

void i18n.use(initReactI18next).init({
  lng: currentLocale(),
  fallbackLng: DEFAULT_LOCALE,
  supportedLngs: supportedLocales,
  resources: Object.fromEntries(Object.entries(localeCatalogs).map(([locale, translation]) => [locale, { translation, "reports-reviews-domain": reportsReviewsMessages[locale]!, "execution-domain": executionMessages[locale as keyof typeof executionMessages], "host-repository-domain": hostRepositoryMessages[locale as keyof typeof hostRepositoryMessages], "insight-domain": insightResources[locale]!, "resource-domain": resourceMessages[locale as keyof typeof resourceMessages], "typed-domain": Object.fromEntries(Object.entries(typedMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])), "common-domain": commonMessages[locale as keyof typeof commonMessages], theme: themeMessages[locale as keyof typeof themeMessages] }])),
  interpolation: { escapeValue: false },
  initAsync: false,
});

function syncLanguage() {
  const locale = currentLocale();
  void i18n.changeLanguage(locale);
  document.documentElement.lang = locale;
  document.documentElement.dir = locale.startsWith("ar-") ? "rtl" : "ltr";
  document.title = i18n.t("app.title");
}
window.addEventListener(LOCALE_EVENT, syncLanguage);
window.addEventListener("popstate", syncLanguage);
syncLanguage();

export { i18n };

export function executionText(key: string, values?: Record<string, unknown>): string {
  return i18n.t(key, { ns: "execution-domain", keySeparator: false, ...values });
}

export function hostRepositoryText(key: string, values?: Record<string, unknown>): string {
  return i18n.t(key, { ns: "host-repository-domain", keySeparator: false, ...values });
}

export function insightText(key: string, values?: Record<string, unknown>): string {
  return i18n.t(key, { ns: "insight-domain", keySeparator: false, defaultValue: key, ...values });
}

export function typedText(key: keyof typeof typedMessages, values?: Record<string, unknown>): string {
  return i18n.t(key, { ns: "typed-domain", keySeparator: false, ...values });
}

/** Common UI text. Configuration keys are resolved only at render time. */
export function commonText(key: string, values?: Record<string, unknown>): string {
  return i18n.t(key, { ns: "common-domain", nsSeparator: ":", keySeparator: false, ...values });
}
export function useCommonTranslation() {
  return useTranslation("common-domain");
}
export function configText(value: string | undefined, key?: string): string {
  return key ? commonText(key) : value ?? "";
}
