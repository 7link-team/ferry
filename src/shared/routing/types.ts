import type { Locale } from "../i18n/locale";

export interface Route {
  locale?: Locale;
  workspace: string;
  group: string;
  item: string;
  id?: string;
  mode: "list" | "new" | "view" | "edit" | "workspace";
}


export type Navigation = {
  href: (route: Partial<Route>, query?: Record<string, string>) => string;
  navigate: (route: Partial<Route>, query?: Record<string, string>, options?: { replace?: boolean }) => void;
};
