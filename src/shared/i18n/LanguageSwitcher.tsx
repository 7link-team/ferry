import { useTranslation } from "react-i18next";
import "./i18n";
import { currentLocale,localeNames,setLocale,supportedLocales,type Locale } from "./locale";

/** Native select keeps keyboard, touch and assistive-technology behavior consistent. */
export function LanguageSwitcher({ floating = false }: { floating?: boolean }) {
  const { t } = useTranslation();
  return <label className={floating ? "fixed end-3 top-2 z-40 flex items-center gap-2 rounded-md bg-card px-2 text-sm" : "flex shrink-0 items-center gap-2 text-sm"}>
    <span className="sr-only">{t("language.label")}</span>
    <select aria-label={t("language.label")} value={currentLocale()} onChange={event => setLocale(event.target.value as Locale)}
      className="h-11 max-w-40 rounded-md border border-border bg-card px-2 text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring">
      {supportedLocales.map(locale => <option key={locale} value={locale} lang={locale}>{localeNames[locale]}</option>)}
    </select>
  </label>;
}
