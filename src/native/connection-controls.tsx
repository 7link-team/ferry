import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { LanguageSwitcher } from "../shared/i18n/LanguageSwitcher";
import { currentLocale, LOCALE_EVENT } from "../shared/i18n/locale";
import { ThemeSwitcher } from "../shared/theme/ThemeSwitcher";
import { initTheme } from "../shared/theme/theme";
import "../../desktop/ui/connection.css";

// This document stays at the exact bundled URL required by native IPC guards.
initTheme();
document.getElementById("connection-controls")!.dataset.ready = "true";
const language = document.getElementById("connection-language") as HTMLSelectElement;
const syncLocale = () => {
  language.value = currentLocale();
  language.dispatchEvent(new Event("change"));
};
syncLocale();
window.addEventListener(LOCALE_EVENT, syncLocale);
flushSync(() => createRoot(document.getElementById("connection-controls")!).render(
  <><LanguageSwitcher showLabel preservePath /><ThemeSwitcher showLabel /></>,
));

// Native handlers own form state; React owns only the shared preference controls.
void import("../../desktop/ui/launcher.js");
