import type { ReactNode } from "react";
import { listResource } from "../api/resource";
import { api } from "../api/api";
import { commonText, optionText } from "../i18n/i18n";
import { currentLocale } from "../i18n/locale";
import type { Navigation } from "../routing/types";
import type { FormConfig, ListConfig, Option } from "./types";

export type AnyRow = any;

export const list = <T,>(config: ListConfig<T>): ListConfig<AnyRow> => config;

export const form = <T,>(config: FormConfig<T>): FormConfig<AnyRow> => config;

export const label = (options: Option[], value: string) => {
  const option = options.find((option) => option.value === value);
  return option ? optionText(option) : value;
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
  <T extends { id: string; name: string } = { id: string; name: string }>(workspace: string, resource: string, render?: (row: T) => ReactNode, conditions: Record<string, string> = {}) => async (): Promise<Option[]> => {
    const items: Option[] = [];
    for (let page = 1; ; page += 1) {
      const result = await listResource<T>(
        workspace,
        resource,
        { ...conditions, page, page_size: 200 },
      );
      items.push(
        ...result.items.map((row) => ({ value: row.id, label: row.name, ...(render ? { content: render(row) } : {}) })),
      );
      if (!result.items.length || page * result.page_size >= result.total)
        return items;
    }
  };

/** Builtin presentation dictionaries are readable without maintenance permissions. */
export const dictionaryOptions = (workspace: string, key: string) => async (): Promise<Option[]> => {
  const preferences = await api<{ dictionaries: Record<string, Option[]> }>(`/workspaces/${workspace}/preferences`);
  return preferences.dictionaries[key] ?? [];
};

export const cronDate = (value: number) =>
  value ? new Date(value).toLocaleString(currentLocale()) : "—";

export type ResourceConfigContext = {
  renderTable?: (config: ListConfig<Record<string, unknown>>, refreshKey?: number) => ReactNode;
  workspace: string;
  permissions?: Set<string> | undefined;
  onChanged?: (() => void) | undefined;
  navigate: Navigation["navigate"];
  href?: Navigation["href"] | undefined;
  dictionaries?: Record<string, Option[]>;
};
