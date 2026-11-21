import { useSyncExternalStore } from "react";
import { ServerCog } from "lucide-react";
import { commonText, useCommonTranslation } from "../shared/i18n/i18n";
import { Button } from "../shared/ui/button";

function subscribe(listener: () => void) {
  window.addEventListener("client-shell:ready", listener);
  return () => window.removeEventListener("client-shell:ready", listener);
}

function isMobileClient() {
  return document.documentElement.dataset.nativeClient === "mobile";
}

/** Navigation intent only. Remote content never receives native command access. */
export function ClientConnection() {
  useCommonTranslation();
  const mobile = useSyncExternalStore(subscribe, isMobileClient, () => false);
  if (!mobile) return null;
  const label = commonText("app.serverConnection");
  return <Button asChild variant="ghost" size="icon" className="size-11 shrink-0">
    <a href="app-settings://connection" aria-label={label} title={label} data-client-connection-entry>
      <ServerCog aria-hidden="true" />
    </a>
  </Button>;
}
