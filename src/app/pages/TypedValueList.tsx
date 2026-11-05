import { useCallback, useEffect, useMemo, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { typedText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { ListConfig } from "../../shared/resources/types";
import type { TypedValueRow } from "../../features/maintenance/TypedValueEditor";
import { categoryLabel } from "../../features/maintenance/typed-resources";
import { ResourceTable } from "../resources/ResourceTable";
import { useQueryValue } from "../router";
import { GroupSidebar, type GroupNode } from "./GroupSidebar";

export function TypedValueList({ config, workspace, permissions }: {
 config: ListConfig<TypedValueRow>; workspace: string; permissions: Set<string>;
}) {
 const resource = config.resource as "settings" | "dictionaries";
 const [category, setCategory] = useQueryValue("category", "");
 const [groups, setGroups] = useState<GroupNode[]>([]);
 const [total, setTotal] = useState<number>();
 const [error, setError] = useState("");
 const [revision, setRevision] = useState(0);
 const refreshGroups = useCallback(() => setRevision(value => value + 1), []);
 const locale = currentLocale();
 useEffect(() => {
  const controller = new AbortController();
  void api<{ items: { category: string; count: number }[]; total: number }>(`${base(workspace, resource)}/categories`, { signal: controller.signal })
   .then(result => {
    setGroups(result.items.map(item => ({ id: item.category, name: categoryLabel(item.category), count: item.count, description: "", version: 1 })));
    setTotal(result.total); setError("");
   }).catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
  return () => controller.abort();
 }, [workspace, resource, revision, locale]);
 const table = useMemo(() => ({ ...config, fixed: { ...config.fixed, ...(category ? { category } : {}) }, filters: config.filters?.filter(filter => filter.key !== "category") ?? [] }), [config, category]);
 const select = (value: string) => {
  const query = new URLSearchParams(location.search);
  query.delete("page"); query.delete("at");
  history.replaceState(history.state, "", `${location.pathname}${query.size ? `?${query}` : ""}`);
  setCategory(value);
 };
 return <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 sm:flex-row">
  <GroupSidebar workspace={workspace} permissions={permissions} resource={resource} noun={typedText("category")} active={category} onSelect={select} groups={groups} {...(total === undefined ? {} : { total })} />
  <div className="flex min-h-0 min-w-0 flex-1 flex-col">
   {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
   <ResourceTable config={table} workspace={workspace} permissions={permissions} onLoaded={refreshGroups} />
  </div>
 </div>;
}
