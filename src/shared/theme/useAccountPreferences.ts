import { useEffect } from "react";
import { toast } from "sonner";
import { api, errorText } from "../api/api";
import { LOCALE_PREFERENCE_EVENT, setLocale, supportedLocales, type Locale } from "../i18n/locale";
import { restoreTheme, THEME_PREFERENCE_EVENT, type ThemeState } from "./theme";

type Preferences = { theme?: ThemeState | null; locale?: Locale | null };

/** One subscription per authenticated account. Loading never writes back stale local values. */
export function useAccountPreferences(userId?: string) {
  useEffect(() => {
    if (!userId) return;
    let live = true;
    const changed = new Set<keyof Preferences>();
    const controller = new AbortController();
    let writes = Promise.resolve();
    const save = (patch: Preferences) => {
      for (const key of Object.keys(patch) as (keyof Preferences)[]) changed.add(key);
      // Preserve edit order, including changes while the initial read is pending.
      writes = writes.then(async () => {
        if (!live) return;
        try { await api("/me/preferences", { method: "PATCH", body: JSON.stringify(patch), signal: controller.signal }); }
        catch (error) { if (live) toast.error(errorText(error)); }
      });
    };
    const themeChanged = (event: Event) => save({ theme: (event as CustomEvent<ThemeState>).detail });
    const localeChanged = (event: Event) => save({ locale: (event as CustomEvent<Locale>).detail });
    window.addEventListener(THEME_PREFERENCE_EVENT, themeChanged);
    window.addEventListener(LOCALE_PREFERENCE_EVENT, localeChanged);
    void api<Preferences>("/me/preferences", { signal: controller.signal }).then(value => {
      if (!live) return;
      if (!changed.has("theme")) restoreTheme(value.theme);
      if (!changed.has("locale") && value.locale && supportedLocales.includes(value.locale)) setLocale(value.locale, false);
    }).catch(error => { if (live) toast.error(errorText(error)); });
    return () => {
      live = false;
      controller.abort();
      window.removeEventListener(THEME_PREFERENCE_EVENT, themeChanged);
      window.removeEventListener(LOCALE_PREFERENCE_EVENT, localeChanged);
    };
  }, [userId]);
}
