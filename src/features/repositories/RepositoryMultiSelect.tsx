import { useState } from "react";
import { hostRepositoryText, workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";

export type CloneRepository = { url: string; name: string; full_name?: string };
export function repositoryGroups(rows: readonly CloneRepository[]) {
  return [...new Set(rows.flatMap(row => {
    const parts = (row.full_name || row.name).split("/"); parts.pop();
    return parts.map((_, index) => parts.slice(0, index + 1).join("/"));
  }))].sort();
}
/** Selection contains explicit repositories, never a silently changing remote group. */
export function RepositoryMultiSelect({ rows, selected, change, disabled = false }: {
  rows: readonly CloneRepository[]; selected: readonly CloneRepository[];
  change: (rows: CloneRepository[]) => void; disabled?: boolean;
}) {
  const [search, setSearch] = useState("");
  const [limit, setLimit] = useState(100);
  const groups = repositoryGroups(rows);
  const visible = rows.filter(row => (row.full_name || row.name).toLocaleLowerCase().includes(search.toLocaleLowerCase()));
  const toggle = (choices: readonly CloneRepository[], checked: boolean) => {
    const urls = new Set(choices.map(row => row.url));
    change(checked ? [...selected, ...choices.filter(row => !selected.some(old => old.url === row.url))] : selected.filter(row => !urls.has(row.url)));
  };
  const checkbox = (label: string, choices: readonly CloneRepository[]) => <label key={label} className="flex min-h-11 min-w-0 cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-sm hover:bg-muted/50">
    <input type="checkbox" className="size-4 shrink-0 accent-primary" disabled={disabled || !choices.length} checked={!!choices.length && choices.every(row => selected.some(old => old.url === row.url))} onChange={event => toggle(choices, event.target.checked)} />
    <span className="min-w-0 break-all">{label}</span>
  </label>;
  return <div className="min-w-0 space-y-3">
    <Input aria-label={t("actions.searchRepositories")} placeholder={t("actions.searchRepositories")} value={search} disabled={disabled} onChange={event => { setSearch(event.target.value); setLimit(100); }} />
    {!!groups.length && <details className="rounded-md border p-3"><summary className="cursor-pointer text-sm font-medium">{t("actions.selectGroups")}</summary><div className="mt-2 grid max-h-56 gap-2 overflow-y-auto sm:grid-cols-2">{groups.map(group => {
      const choices = rows.filter(row => (row.full_name || row.name).startsWith(group + "/"));
      return checkbox(`${group} · ${choices.length}`, choices);
    })}</div></details>}
    {checkbox(t("actions.selectResults").replace("{{count}}", String(visible.length)), visible)}
    <div className="grid max-h-64 gap-2 overflow-y-auto sm:grid-cols-2">{visible.slice(0, limit).map(row => checkbox(row.full_name || row.name, [row]))}</div>
    {visible.length > limit && <Button type="button" variant="outline" onClick={() => setLimit(value => value + 100)}>{t("integration.loadOptions")}</Button>}
    <div className="flex flex-wrap items-center justify-between gap-2 text-sm"><span>{t("actions.selectedRepositories").replace("{{count}}", String(selected.length))}</span><Button type="button" variant="ghost" disabled={disabled || !selected.length} onClick={() => change([])}>{t("actions.clearSelection")}</Button></div>
    {!visible.length && <p role="status" className="text-sm text-muted-foreground">{hostRepositoryText("repository.noVisibleRepositories")}</p>}
    {selected.some(row => !rows.some(visible => visible.url === row.url)) && <ul className="space-y-1 text-sm">{selected.filter(row => !rows.some(visible => visible.url === row.url)).map(row => <li key={row.url} className="break-all">{row.full_name || row.name}</li>)}</ul>}
  </div>;
}
