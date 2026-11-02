import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import "./shared/i18n/i18n";
import { initLocale } from "./shared/i18n/locale";
import { initTheme } from "./shared/theme/theme";
import { Toaster } from "./shared/ui/sonner";
import "./style.css";

// Before React mounts, so the first paint is already the chosen theme and
// there is no flash of the wrong palette.
initTheme();
initLocale();

createRoot(document.getElementById("root")!).render(
  <>
    <App />
    <Toaster />
  </>,
);
