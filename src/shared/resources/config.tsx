import { listResource } from "../api/resource";
import { commonText, configText } from "../i18n/i18n";
import { currentLocale } from "../i18n/locale";
import type { Navigation } from "../routing/types";
import type { FormConfig, ListConfig, Option } from "./types";

export type AnyRow = any;

export const list = <T,>(config: ListConfig<T>): ListConfig<AnyRow> => config;

export const form = <T,>(config: FormConfig<T>): FormConfig<AnyRow> => config;

export const label = (options: Option[], value: string) => {
  const option = options.find((option) => option.value === value);
  return option ? configText(option.label, option.labelKey) : value;
};

export function ago(at: number) {
  if (!at) return "—";
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60)
    return commonText("resource-domain:relative.seconds", { count: seconds });
  if (seconds < 3600)
    return commonText("resource-domain:relative.minutes", {
      count: Math.floor(seconds / 60),
    });
  if (seconds < 86400)
    return commonText("resource-domain:relative.hours", {
      count: Math.floor(seconds / 3600),
    });
  return commonText("resource-domain:relative.days", {
    count: Math.floor(seconds / 86400),
  });
}

export const namedOptions =
  (workspace: string, resource: string) => async (): Promise<Option[]> => {
    const items: Option[] = [];
    for (let page = 1; ; page += 1) {
      const result = await listResource<{ id: string; name: string }>(
        workspace,
        resource,
        { page, page_size: 200 },
      );
      items.push(
        ...result.items.map((row) => ({ value: row.id, label: row.name })),
      );
      if (!result.items.length || page * result.page_size >= result.total)
        return items;
    }
  };

export const cronDate = (value: number) =>
  value ? new Date(value).toLocaleString(currentLocale()) : "—";

export type ResourceConfigContext = {
  workspace: string;
  permissions?: Set<string> | undefined;
  onChanged?: (() => void) | undefined;
  navigate: Navigation["navigate"];
};
