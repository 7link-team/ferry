import { createContext, useContext } from "react";

export interface SiteSettings {
  site_name: string;
  site_title_prefix_enabled: boolean;
  site_title_visible_prefix: string;
  site_title_hidden_prefix: string;
}
export const defaultSiteSettings: SiteSettings = {
  site_name: "",
  site_title_prefix_enabled: false,
  site_title_visible_prefix: "☀️",
  site_title_hidden_prefix: "🌙",
};
export const SiteSettingsContext = createContext<SiteSettings>(defaultSiteSettings);
export const useSiteSettings = () => useContext(SiteSettingsContext);
