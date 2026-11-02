import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Toaster } from "@/components/ui/sonner";
import { initTheme } from "./theme";
import { initLocale } from "./locale";
import "./i18n";
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
