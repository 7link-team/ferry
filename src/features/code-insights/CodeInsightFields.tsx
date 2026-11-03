import { useEffect, useState } from "react";
import { ChevronDown, GitBranch, Plus, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { AutoField } from "../../shared/forms/auto-form";
import { SelectRefresh } from "../../shared/forms/SelectRefresh";
import { insightText } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuTrigger } from "../../shared/ui/dropdown-menu";
import {
  ConnectionRepositoryFields,
  useConnectionRepositories,
} from "../repositories/ConnectionRepositoryFields";

export interface InsightRepository {
  connection_id: string;
  repository: string;
  pipeline_id: string;
  all_repositories?: boolean;
  connection_name?: string;
  pipeline_name?: string;
  provider?: string;
}
type Props = {
  navigation: Navigation;
  workspace: string;
  value: unknown;
  readOnly: boolean;
  permissions: Set<string>;
  onChange: (value: unknown) => void;
  onValidityChange: (blocked: boolean) => void;
};

/** One shared form field manages memberships; connection browsing is reused from checkout. */
export function CodeInsightRepositories({
  navigation,
  workspace,
  value,
  readOnly,
  permissions,
  onChange,
  onValidityChange,
}: Props & { enabled?: boolean }) {
  useTranslation("insight-domain");
  const selected = Array.isArray(value) ? (value as InsightRepository[]) : [];
  const canSelect = permissions.has("repositories.read");
  const [choice, setChoice] = useState({ connection_id: "", url: "" });
  const [group, setGroup] = useState("");
  const [search, setSearch] = useState("");
  useEffect(() => setSearch(""), [choice.connection_id, group]);
  const data = useConnectionRepositories(
    workspace,
    choice.connection_id,
    !readOnly && canSelect,
    true,
  );
  const incomplete =
    !selected.length ||
    selected.some(
      (row) =>
        !row.connection_id ||
        (!row.all_repositories && !row.repository.trim()),
    );
  useEffect(() => {
    onValidityChange(!readOnly && incomplete);
    return () => onValidityChange(false);
  }, [readOnly, incomplete, onValidityChange]);
  const add = () => {
    const remote = data.repositories.find((row) => row.url === choice.url);
    const repository =
      data.connection?.provider === "git"
        ? choice.url.trim()
        : (remote?.full_name ?? "");
    if (
      !data.connection ||
      !repository ||
      selected.some(
        (row) =>
          row.connection_id === choice.connection_id &&
          row.repository === repository,
      )
    )
      return;
    onChange([
      ...selected,
      { connection_id: choice.connection_id, repository, pipeline_id: "" },
    ]);
    setChoice((current) => ({ ...current, url: "" }));
  };
  const visible = data.repositories.filter(remote => (!group || remote.full_name.startsWith(group + "/")) && remote.full_name.toLowerCase().includes(search.trim().toLowerCase()));
  const matches = (row: InsightRepository, remote: typeof data.repositories[number]) => row.connection_id === choice.connection_id && (row.repository === remote.full_name || row.repository === remote.url);
  const allVisible = visible.length > 0 && visible.every(remote => selected.some(row => matches(row, remote)));
  const toggle = (remotes: typeof data.repositories, checked: boolean) => {
    if (!data.connection || data.loading || data.error) return;
    onChange(checked ? [...selected, ...remotes.filter(remote => !selected.some(row => matches(row, remote))).map(remote => ({ connection_id: choice.connection_id, repository: remote.full_name, pipeline_id: "", provider: data.connection!.provider }))] : selected.filter(row => !remotes.some(remote => matches(row, remote))));
  };
  return (
    <section
      aria-label={insightText("洞察仓库")}
      className="space-y-4 sm:col-span-2"
    >
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-medium">
          {insightText("选择仓库")}
          <span className="ml-1 text-destructive" aria-hidden="true">
            *
          </span>
        </h3>
        <span className="rounded-md bg-secondary px-2 py-1 text-xs tabular-nums text-secondary-foreground">
          {insightText("selectedRepositories", { count: selected.length })}
        </span>
      </div>
      {selected.length > 0 && (
        <details open={readOnly} className="rounded-lg border p-3">
        <summary className="cursor-pointer text-sm font-medium">{insightText("selectedRepositories", { count: selected.length })}</summary>
        <ul className="divide-y rounded-lg border">
          {selected.map((row, index) => (
            <li
              key={`${row.connection_id}/${row.repository}`}
              className="space-y-3 p-4"
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 items-center gap-3">
                  <GitBranch
                    aria-hidden="true"
                    className="size-4 shrink-0 text-muted-foreground"
                  />
                  <p className="min-w-0 break-all text-sm">
                    <span className="block text-xs text-muted-foreground">
                      {row.connection_name ||
                        data.connections.find(
                          (connection) => connection.id === row.connection_id,
                        )?.name ||
                        insightText("仓库连接")}
                    </span>
                    <span className="font-medium">
                      {row.all_repositories
                        ? insightText("全部可见仓库")
                        : row.repository}
                    </span>
                  </p>
                </div>
                {!readOnly && canSelect && (
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    aria-label={insightText("removeRepository", {
                      repository: row.repository,
                    })}
                    onClick={() =>
                      onChange(
                        selected.filter((_, position) => position !== index),
                      )
                    }
                  >
                    {insightText("移除")}
                  </Button>
                )}
              </div>
            </li>
          ))}
        </ul>
        </details>
      )}
      {!readOnly && canSelect && (
        <div className="grid min-w-0 gap-4 rounded-lg bg-muted/40 p-4 sm:grid-cols-2">
          <ConnectionRepositoryFields
            navigation={navigation}
            workspace={workspace}
            data={data}
            connectionId={choice.connection_id}
            repositoryUrl={choice.url}
            repositoryLabel={insightText("添加洞察仓库")}
            repositoryPicker={<DropdownMenu>
              <SelectRefresh label={insightText("刷新仓库")} loading={data.loading} onRefresh={() => data.setRevision(value => value + 1)}>
                <DropdownMenuTrigger asChild><Button type="button" variant="outline" className="h-10 w-full justify-between font-normal" aria-label={insightText("选择仓库")} disabled={data.loading || !!data.error}>
                  <span className="truncate">{insightText("selectedRepositories", { count: selected.filter(row => row.connection_id === choice.connection_id).length })}</span><ChevronDown aria-hidden="true" className="size-4 shrink-0 opacity-50" />
                </Button></DropdownMenuTrigger>
              </SelectRefresh>
              <DropdownMenuContent align="start" className="w-[min(32rem,calc(100vw-3rem))] max-h-[min(28rem,65dvh)] overflow-y-auto p-1">
                <div className="sticky top-0 z-10 space-y-2 border-b bg-popover p-2">
                  <Input aria-label={insightText("搜索仓库")} placeholder={insightText("搜索仓库")} value={search} onChange={event => setSearch(event.target.value)} onKeyDown={event => { event.stopPropagation(); if (event.key === "ArrowDown") { event.preventDefault(); event.currentTarget.closest('[role="menu"]')?.querySelector<HTMLElement>('[role="menuitemcheckbox"]')?.focus(); } }} />
                  <DropdownMenuCheckboxItem checked={allVisible} disabled={!visible.length} onSelect={event => event.preventDefault()} onCheckedChange={() => toggle(visible, !allVisible)}>{insightText("选择当前结果（{{count}}）", { count: visible.length })}</DropdownMenuCheckboxItem>
                </div>
                {visible.map(remote => <DropdownMenuCheckboxItem key={remote.url} checked={selected.some(row => matches(row, remote))} onSelect={event => event.preventDefault()} onCheckedChange={checked => toggle([remote], !!checked)} className="break-all py-2">{remote.full_name}</DropdownMenuCheckboxItem>)}
                {!visible.length && <p role="status" className="p-4 text-sm text-muted-foreground">{insightText("暂无匹配仓库")}</p>}
              </DropdownMenuContent>
            </DropdownMenu>}
            gitAction={<Button type="button" size="sm" variant="outline" disabled={!choice.url.trim() || !data.connection || data.loading || !!data.error} onClick={add}><Plus aria-hidden="true" className="size-4" />{insightText("加入洞察")}</Button>}
            group={group}
            onGroupChange={value => { setGroup(value); setChoice(current => ({ ...current, url: "" })); }}
            onChange={(selection) => {
              if (selection.connection_id !== choice.connection_id) setGroup("");
              setChoice({
                connection_id: selection.connection_id,
                url: selection.url,
              });
            }}
          />
        </div>
      )}
      {!readOnly && !canSelect && (
        <p className="text-sm text-muted-foreground">
          {insightText("更改仓库范围需要仓库读取权限。")}
        </p>
      )}
    </section>
  );
}

/** Edit identities as rows while preserving existing contributor IDs and metadata. */
export function CodeInsightAliases({
  value,
  readOnly,
  onChange,
  onValidityChange,
}: Omit<Props, "workspace" | "permissions">) {
  useTranslation("insight-domain");
  const aliases = Array.isArray(value)
    ? (value as Record<string, unknown>[])
    : [];
  const incomplete = aliases.some(
    (alias) =>
      !String(alias.nickname ?? "").trim() ||
      (!String(alias.name ?? "").trim() && !String(alias.email ?? "").trim()),
  );
  useEffect(() => {
    onValidityChange(!readOnly && incomplete);
    return () => onValidityChange(false);
  }, [readOnly, incomplete, onValidityChange]);
  const update = (index: number, key: string, text: string) =>
    onChange(
      aliases.map((alias, position) =>
        position === index ? { ...alias, [key]: text } : alias,
      ),
    );
  return (
    <section
      aria-label={insightText("作者昵称映射")}
      className="space-y-4 sm:col-span-2"
    >
      <div className="flex flex-wrap items-start justify-between gap-3"><p className="min-w-0 flex-1 text-sm leading-6 text-muted-foreground">
        {insightText(
          "将不同提交姓名或邮箱映射到同一昵称，统一贡献统计。仅影响当前洞察，不会重新采集代码。",
        )}
      </p>
      {!readOnly && <Button type="button" size="sm" variant="outline" onClick={() => onChange([...aliases, { name: "", email: "", nickname: "" }])}><Plus aria-hidden="true" className="size-4" />{insightText("添加作者映射")}</Button>}
      </div>
      {aliases.length === 0 && (
        <p
          role="status"
          className="rounded-lg border border-dashed p-5 text-sm text-muted-foreground"
        >
          {insightText("尚无作者映射，统计将使用原始提交身份。")}
        </p>
      )}
      <div className="space-y-3">
        {aliases.map((alias, index) => (
          <div
            key={index}
            className="grid min-w-0 items-start gap-4 rounded-lg border p-4 sm:grid-cols-2 lg:grid-cols-[1fr_1fr_1fr_auto]"
          >
            {(
              [
                ["name", "提交姓名"],
                ["email", "提交邮箱"],
                ["nickname", "统一昵称"],
              ] as const
            ).map(([key, label]) => (
              <AutoField
                key={key}
                label={insightText(label)}
                required={key === "nickname"}
                className="block min-w-0 space-y-2"
              >
                {readOnly ? (
                  <p className="break-all text-sm">
                    {String(alias[key] || "—")}
                  </p>
                ) : (
                  <Input
                    aria-label={insightText(label) + " " + (index + 1)}
                    value={String(alias[key] ?? "")}
                    required={key === "nickname"}
                    onChange={(event) => update(index, key, event.target.value)}
                  />
                )}
              </AutoField>
            ))}
            {!readOnly && (
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="sm:mt-6"
                aria-label={insightText("removeAlias", { index: index + 1 })}
                onClick={() =>
                  onChange(aliases.filter((_, position) => position !== index))
                }
              >
                <Trash2 aria-hidden="true" className="size-4" />
              </Button>
            )}
          </div>
        ))}
      </div>
      {!readOnly && incomplete && (
        <p role="alert" className="text-sm text-destructive">
          {insightText("每条作者映射须填写姓名或邮箱，以及显示昵称。")}
        </p>
      )}
    </section>
  );
}
