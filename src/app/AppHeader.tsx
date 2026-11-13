import type { AnchorHTMLAttributes, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { BrandIcon } from "../shared/site/BrandIcon";
import { useSiteSettings } from "../shared/site/settings";

/** Shared application chrome; page-specific navigation and actions stay with their owners. */
export function AppHeader({ navigation, actions, home }: {
  navigation?: ReactNode;
  actions: ReactNode;
  home?: AnchorHTMLAttributes<HTMLAnchorElement>;
}) {
  const { t } = useTranslation();
  const site = useSiteSettings();
  const brand = <><BrandIcon /><span className="min-w-0 break-words [overflow-wrap:anywhere]">{site.site_name || t("app.title")}</span></>;
  const brandClass = "flex min-w-0 max-w-full items-center gap-2 font-semibold tracking-tight sm:max-w-xs";
  return <header data-app-header className="flex min-h-14 min-w-0 flex-wrap items-center gap-x-4 gap-y-2 border-b border-border bg-background px-4 py-2">
    {home ? <a {...home} className={brandClass}>{brand}</a> : <div className={brandClass}>{brand}</div>}
    {navigation}
    <div className="ms-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-2">{actions}</div>
  </header>;
}
