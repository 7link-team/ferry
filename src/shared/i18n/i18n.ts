import notificationMessages from "./locales/notifications.json";
import i18n from "i18next";
import { initReactI18next,useTranslation } from "react-i18next";
import { currentLocale,DEFAULT_LOCALE,LOCALE_EVENT,localeCatalogs,supportedLocales } from "./locale";
import commonMessages from "./locales/common-domain.json";
import executionMessages from "./locales/execution-domain.json";
import hostRepositoryMessages from "./locales/host-repository-domain.json";
import insightMessages from "./locales/insight-domain.json";
import insightZh from "./locales/insight-zh.json";
import resourceMessages from "./locales/resource-domain.json";
import themeMessages from "./locales/theme.json";
import typedMessages from "./locales/typed-domain.json";
import workflowMessages from "./locales/workflow-domain.json";
import agentMessages from "./locales/agents.json";
import modelMessages from "./locales/models.json";
import conversationMessages from "./locales/conversations.json";
import holeMessages from "./locales/holes.json";
import otpMessages from "./locales/otp.json";
import identityMessages from "./locales/identity.json";

// Column order is explicit so adding/reordering supported locales cannot change translations.
const typedLocales = ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"];
const insightLocales = ["en-US", "zh-TW", "ja-JP", "ko-KR", "ms-MY"];
const insightResources = Object.fromEntries(typedLocales.map(locale => [locale, Object.fromEntries(Object.entries(insightMessages).map(([key, values]) => [key, locale === "zh-CN" ? insightZh[key as keyof typeof insightZh] ?? key : values[insightLocales.indexOf(locale)]!]))]));

void i18n.use(initReactI18next).init({
  lng: currentLocale(),
  fallbackLng: DEFAULT_LOCALE,
  supportedLngs: supportedLocales,
  resources: Object.fromEntries(Object.entries(localeCatalogs).map(([locale, translation]) => [locale, { translation, "execution-domain": executionMessages[locale as keyof typeof executionMessages], "host-repository-domain": hostRepositoryMessages[locale as keyof typeof hostRepositoryMessages], "insight-domain": insightResources[locale]!, "resource-domain": resourceMessages[locale as keyof typeof resourceMessages], "typed-domain": Object.fromEntries(Object.entries(typedMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])), "common-domain": commonMessages[locale as keyof typeof commonMessages], theme: themeMessages[locale as keyof typeof themeMessages] }])),
  interpolation: { escapeValue: false },
  initAsync: false,
});

for (const locale of typedLocales) i18n.addResourceBundle(locale, "workflow-domain", Object.fromEntries(Object.entries(workflowMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));
for (const locale of typedLocales) i18n.addResourceBundle(locale, "agents", Object.fromEntries(Object.entries(agentMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));
for (const locale of typedLocales) i18n.addResourceBundle(locale, "models", Object.fromEntries(Object.entries(modelMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));
for (const locale of typedLocales) i18n.addResourceBundle(locale, "conversations", Object.fromEntries(Object.entries(conversationMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));
for (const locale of typedLocales) i18n.addResourceBundle(locale, "notifications", Object.fromEntries(Object.entries(notificationMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));
for (const locale of typedLocales) i18n.addResourceBundle(locale, "holes", Object.fromEntries(Object.entries(holeMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));
for (const locale of typedLocales) i18n.addResourceBundle(locale, "otp", Object.fromEntries(Object.entries(otpMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));

for (const locale of typedLocales) i18n.addResourceBundle(locale, "identity", Object.fromEntries(Object.entries(identityMessages).map(([key, values]) => [key, values[typedLocales.indexOf(locale)]])));

export function workflowText(key: keyof typeof workflowMessages): string {
  return i18n.t(key, { ns: "workflow-domain", keySeparator: false });
}

function syncLanguage() {
  const locale = currentLocale();
  void i18n.changeLanguage(locale);
  document.documentElement.lang = locale;
  document.documentElement.dir = locale.startsWith("ar-") ? "rtl" : "ltr";
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
/** Dictionary translations are presentation only; option.value is never translated. */
export function optionText(option: { label: string; labelKey?: string | undefined; labels?: Record<string, string> }): string {
  return option.labels?.[i18n.resolvedLanguage ?? currentLocale()] ?? option.labels?.[DEFAULT_LOCALE] ?? configText(option.label, option.labelKey);
}
