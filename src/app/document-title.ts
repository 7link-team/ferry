import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { commonText, i18n } from "../shared/i18n/i18n";
import type { Route } from "../shared/routing/types";
import { findItem } from "./nav";
import { useSiteSettings } from "../shared/site/settings";

function pageTitle(route: Route): string {
  switch (route.workspace) {
    case "_login": return commonText("auth.signIn");
    case "_register": return commonText("auth.register");
    case "_initialize": return commonText("auth.setupTitle");
    case "_workspaces": return i18n.t("app.workspaces");
    case "_account": return i18n.t("app.account");
    case "_platform": {
      const labels: Record<string, string> = {
        workspaces: "platform.workspaces", users: "platform.users",
        settings: "platform.settings", monitor: "monitor.title",
        "server-nodes": "platform.serverNodes",
      };
      return labels[route.item] ? commonText(`resource-domain:${labels[route.item]}`) : i18n.t("app.platform");
    }
    default: return findItem(route.group, route.item)?.label ?? "";
  }
}

/** One owner for the title; visibility changes never accumulate emoji prefixes. */
export function useDocumentTitle(route: Route) {
  const { t } = useTranslation();
  const site = useSiteSettings();
  const page = pageTitle(route);
  const action = route.mode === "new" ? commonText("common.create")
    : route.mode === "edit" ? commonText("common.edit") : "";
  const name = site.site_name || t("app.title");
  const title = page ? `${action ? `${action} · ` : ""}${page} | ${name}` : name;
  useEffect(() => {
    const update = () => {
      const prefix = site.site_title_prefix_enabled
        ? document.visibilityState === "hidden" ? site.site_title_hidden_prefix : site.site_title_visible_prefix : "";
      document.title = prefix ? `${prefix} ${title}` : title;
    };
    update();
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, [title, site.site_title_prefix_enabled, site.site_title_visible_prefix, site.site_title_hidden_prefix]);
}
