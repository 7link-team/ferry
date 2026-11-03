import { commonText,configText,useCommonTranslation } from "../../shared/i18n/i18n";
/**
 * The one list implementation. A resource page is a `ListConfig`, never a
 * hand-written table — see CLAUDE.md §12. This component owns:
 *
 *   filter state in the URL · pagination · auto-refresh · row selection ·
 *   import/export · loading / empty / error states · permission filtering ·
 *   the sticky frame (header, first column, last column, footer)
 *
 * Sticky frame rules, because they are easy to break:
 *   - The page fills its parent's remaining height (`flex-1`) and is a
 *     3-row grid (toolbar / scroll / footer). `min-h-0` and `min-w-0` are
 *     mandatory on the grid and the scroll row. Grid items default to
 *     `min-height: auto`, so without it nothing shrinks, the whole page
 *     scrolls, and all four edges come unstuck.
 *   - Sticky cells are transparent by default; each needs its own background
 *     or rows show through while scrolling.
 *   - Header z above body, corner cells above the header.
 *   - The footer sits outside the scroll container. As a sticky child it
 *     jitters inside iOS Safari's rubber-band scroll.
 */
import { cn } from "cn";
import {
ArrowUp,
Cable,
Columns3,
ChevronRight,
Copy,
Download,
Ellipsis,
Eye,
Folder,
FolderOpen,
FolderTree,
GitBranch,
History,
LayoutTemplate,
List,
Loader2,
Pencil,
Play,
PlugZap,
Plus,
RefreshCw,
RotateCw,
Search,
ShieldCheck,
Trash2,
Users,
X,
} from "lucide-react";
import { useCallback,useEffect,useMemo,useRef,useState,type ReactNode } from "react";
import { errorText } from "../../shared/api/api";
import {
PAGE_SIZES,
REFRESH_INTERVALS,
listResource,
type Envelope,
} from "../../shared/api/resource";
import { AutoField,AutoForm } from "../../shared/forms/auto-form";
import type { ListConfig,Option,RowAction } from "../../shared/resources/types";
import { AutoTable } from "../../shared/tables/auto-table";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import {
DropdownMenu,
DropdownMenuCheckboxItem,
DropdownMenuContent,
DropdownMenuItem,
DropdownMenuTrigger,
} from "../../shared/ui/dropdown-menu";
import { Input } from "../../shared/ui/input";
import {
Select,
SelectContent,
SelectItem,
SelectTrigger,
SelectValue,
} from "../../shared/ui/select";
import { href,navigate,useQueryValue,useRoute,useScrollSync } from "../router";
import { ResourceActionDialog } from "./ResourceForm";
import { ResourceTransfer } from "./ResourceTransfer";

const REFRESH_KEY = (resource: string) => `ferry.refresh.${resource}`;

function actionIcon(label: string): ReactNode {
  const Icon = label.startsWith("查看") ? Eye : label.startsWith("编辑") ? Pencil :
    /删除|移除|清除/.test(label) ? Trash2 : label.includes("复制") ? Copy :
    label.includes("测试") ? PlugZap : /同步|刷新|重启/.test(label) ? RotateCw :
    /执行|运行|触发|部署/.test(label) ? Play : label.includes("目录") ? FolderOpen :
    label.includes("下载") ? Download : label.includes("接入") ? Cable :
    label.includes("模板") ? LayoutTemplate : label.includes("历史") ? History :
    label.includes("角色") ? Users : label.includes("核实") ? ShieldCheck :
    label.includes("恢复") ? RefreshCw : Ellipsis;
  return <Icon className="size-4" />;
}

function actionOrder(label: string) {
  if (label === "查看") return 1;
  if (label === "编辑") return 2;
  if (/^(删除|移除)/.test(label)) return 3;
  return 0;
}

function rowName(row: unknown, fallback: string) {
  const value = row as Record<string, unknown>;
  return String(value.name ?? value.title ?? value.email ?? value.path ?? value.key ?? fallback);
}

function readRefresh(resource: string, fallback = 0) {
  try {
    const stored = localStorage.getItem(REFRESH_KEY(resource));
    if (stored === null) return fallback;
    const saved = Number(stored);
    return (REFRESH_INTERVALS as readonly number[]).includes(saved) ? saved : 0;
  } catch {
    return fallback;
  }
}

/** Options may be static or loaded; both end up here. */
function useOptions(source: Option[] | (() => Promise<Option[]>) | undefined) {
  const [options, setOptions] = useState<Option[]>(Array.isArray(source) ? source : []);
  const [error, setError] = useState("");
  useEffect(() => {
    setError("");
    if (typeof source !== "function") {
      setOptions(Array.isArray(source) ? source : []);
      return;
    }
    let live = true;
    void source().then((loaded) => live && setOptions(loaded)).catch(cause => { if (live) { setOptions([]); setError(errorText(cause)); } });
    return () => {
      live = false;
    };
  }, [source]);
  return { options, error };
}

export function ResourceTable<T>({
  config,
  workspace,
  permissions,
  conditions,
  refreshKey = 0,
  hideToolbar = false,
}: {
  config: ListConfig<T>;
  workspace: string;
  permissions: Set<string>;
  /** A dashboard owns one set of committed conditions for all its reports. */
  conditions?: Record<string, string>;
  refreshKey?: number;
  hideToolbar?: boolean;
}) {
  useCommonTranslation();
  const route = useRoute();
  const queryKey = (key: string) => config.queryPrefix ? `${config.queryPrefix}.${key}` : key;
  const [page, setPage] = useQueryValue(queryKey("page"), 1);
  const [pageSize, setPageSize] = useQueryValue(queryKey("page_size"), config.defaultPageSize ?? 20);
  const [sort, setSort] = useQueryValue(queryKey("sort"), "");
  const [tableView, setTableView] = useQueryValue(queryKey("table_view"), "list");
  const grouped = !!config.grouping && tableView === "groups";
  const [folder, setFolder] = useQueryValue(queryKey("folder"), "");

  // Two layers: `draft` is what the inputs show, the URL is what has been
  // searched. Typing must not refetch on every keystroke, and the URL must
  // stay shareable, so the committed values are the ones in the query string.
  const filterKeys = useMemo(() => (config.filters ?? []).map((f) => f.key), [config.filters]);
  const [committed, setCommitted] = useState<Record<string, string>>(() => readFilters(filterKeys, config.queryPrefix));
  const [draft, setDraft] = useState<Record<string, string>>(committed);
  const [hiddenColumns, setHiddenColumns] = useState<Set<string>>(new Set());

  const [data, setData] = useState<Envelope<T>>();
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [refresh, setRefresh] = useState(() => readRefresh(config.resource, config.defaultRefresh));
  const [busy, setBusy] = useState(false);
  const runningActions = useRef(new Set<string>());
  const [busyActions, setBusyActions] = useState<Set<string>>(new Set());
  const [pendingAction,setPendingAction]=useState<{action:RowAction<T>;row:T}>();
  const [pendingDelete, setPendingDelete] = useState<{ action: string; actionKey?: string | undefined; target: string; run: () => Promise<void> }>();
  const reloadToken = useRef(0);

  const params = useMemo(
    () => ({
      ...conditions,
      ...config.fixed,
      ...Object.fromEntries(Object.entries(committed).filter(([, v]) => v !== "")),
      page: grouped ? 1 : page,
      page_size: grouped ? 100 : pageSize,
      ...(sort ? { sort } : {}),
    }),
    [conditions, config.fixed, committed, grouped ? 1 : page, grouped ? 100 : pageSize, sort, grouped],
  );

  const load = useCallback(
    (signal?: AbortSignal) => {
      setLoading(true);
      const read = async () => {
        if (!grouped) return listResource<T>(workspace, config.resource, params, signal, config.endpoint);
        // Build the namespace directory from every provider page, so folders do
        // not disappear merely because their repositories are on another page.
        const items: T[] = [];
        for (let next = 1; ; next++) {
          const part = await listResource<T>(workspace, config.resource, { ...params, page: next }, signal, config.endpoint);
          items.push(...part.items);
          const more = part.has_more ?? (next * params.page_size < part.total);
          if (!more) break;
          if (!part.items.length) throw new Error(commonText("repository.pageEmpty"));
        }
        return { items, total: items.length, page: 1, page_size: items.length };
      };
      return read()
        .then((envelope) => {
          setData(envelope);
          setError("");
        })
        .catch((cause: unknown) => {
          if (cause instanceof DOMException && cause.name === "AbortError") return;
          setError(errorText(cause));
        })
        .finally(() => setLoading(false));
    },
    [workspace, config.resource, config.endpoint, params, grouped],
  );

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load, reloadToken.current, refreshKey]);

  // Auto-refresh pauses while the tab is hidden: a background tab polling an
  // ops dashboard every 10s is pure waste, and the data is re-read on return.
  useEffect(() => {
    if (!refresh) return;
    let timer = 0;
    const tick = () => {
      if (!document.hidden) void load();
      timer = window.setTimeout(tick, refresh * 1000);
    };
    timer = window.setTimeout(tick, refresh * 1000);
    const wake = () => !document.hidden && void load();
    document.addEventListener("visibilitychange", wake);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", wake);
    };
  }, [refresh, load]);

  const total = data?.total ?? 0;
  const rows = grouped ? [...(data?.items ?? [])].sort((a, b) => config.grouping!.key(a).localeCompare(config.grouping!.key(b))) : data?.items ?? [];
  const directoryRows = useMemo(() => {
    if (!grouped) return [];
    const directories = new Map<string, number>();
    const files: { name: string; path: string; row?: T; count: number }[] = [];
    for (const row of data?.items ?? []) {
      const namespace = config.grouping!.key(row);
      const prefix = folder ? `${folder}/` : "";
      if (namespace !== folder && !namespace.startsWith(prefix)) continue;
      const rest = namespace === folder ? "" : namespace.slice(prefix.length);
      if (rest) {
        const path = prefix + rest.split("/")[0];
        directories.set(path, (directories.get(path) ?? 0) + 1);
      } else files.push({ name: rowName(row, config.rowKey(row)).split('/').at(-1)!, path: config.rowKey(row), row, count: 0 });
    }
    return [...[...directories].sort(([a], [b]) => a.localeCompare(b)).map(([path, count]) => ({ name: path.split('/').at(-1)!, path, count } as { name: string; path: string; row?: T; count: number })), ...files];
  }, [grouped, data, config, folder]);
  const enterFolder = (path: string) => { setFolder(path); setPage(1); };
  useEffect(() => {
    if (grouped && !loading && page > Math.max(1, Math.ceil(directoryRows.length / pageSize))) setPage(1);
  }, [grouped, loading, page, pageSize, directoryRows.length, setPage]);
  const scrollRef = useScrollSync(!loading && rows.length > 0);

  const search = () => {
    setCommitted(draft);
    writeFilters(draft, config.queryPrefix);
    setPage(1);
    setSelected(new Set());
  };
  const clear = () => {
    const empty = Object.fromEntries(filterKeys.map((key) => [key, ""]));
    setDraft(empty);
    setCommitted(empty);
    writeFilters(empty, config.queryPrefix);
    setPage(1);
    setSelected(new Set());
  };
  const reload = () => {
    reloadToken.current += 1;
    void load();
  };

  const visibleActions = (config.actions ?? []).filter(
    (action) => !action.permission || permissions.has(action.permission),
  ).sort((a, b) => actionOrder(a.label) - actionOrder(b.label));
  const visibleBulk = (config.bulk ?? []).filter(
    (action) => !action.permission || permissions.has(action.permission),
  );
  const canCreate = config.toolbar?.create !== false && permissions.has(`${config.permissionResource ?? config.resource}.create`);

  const allOnPageSelected = rows.length > 0 && rows.every((row) => selected.has(config.rowKey(row)));
  const toggleAll = () => {
    const next = new Set(selected);
    for (const row of rows) {
      const key = config.rowKey(row);
      if (allOnPageSelected) next.delete(key);
      else next.add(key);
    }
    setSelected(next);
  };

  const runBulk = async (run: (ids: string[]) => Promise<void>, ids = [...selected]) => {
    setBusy(true);
    try {
      await run(ids);
      setSelected(new Set());
      reload();
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  };



  return (
    <div className={config.summary ? "grid min-h-min min-w-0 flex-1 grid-rows-[auto_minmax(16rem,1fr)] gap-3" : "grid min-h-0 min-w-0 flex-1 grid-rows-[auto_minmax(0,1fr)] gap-3"}>
      {/* ---------------------------------------------------- 搜索过滤 + 工具栏 */}
      {pendingAction&&<ResourceActionDialog action={pendingAction.action} row={pendingAction.row} workspace={workspace} permissions={permissions} onClose={()=>setPendingAction(undefined)} onDone={()=>{setPendingAction(undefined);reload();}} />}
      {pendingDelete && <DeleteConfirmDialog action={configText(pendingDelete.action, pendingDelete.actionKey)} target={pendingDelete.target} onClose={() => setPendingDelete(undefined)} onConfirm={pendingDelete.run} />}
      <div className="flex flex-col gap-3">
        {error && !!rows.length && <p role="alert" className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">{error}</p>}
        {config.filterPresets?.map(group => <div key={group.label} role="group" aria-label={configText(group.label, group.labelKey)} className="flex flex-wrap items-center gap-2">
          <span className="text-sm text-muted-foreground">{configText(group.label, group.labelKey)}</span>
          {group.presets.map(preset => <Button key={preset.label} size="sm" variant="outline" onClick={() => {
            const values = typeof preset.values === "function" ? preset.values() : preset.values;
            const next = { ...committed, ...values };
            setDraft(next); setCommitted(next); writeFilters(next, config.queryPrefix); setPage(1); setSelected(new Set());
          }}>{configText(preset.label, preset.labelKey)}</Button>)}
        </div>)}
        <div data-list-toolbar className="flex flex-wrap items-end gap-3 rounded-md border bg-muted/30 p-3">
        {!!config.filters?.length && (
          <AutoForm
            className="flex flex-wrap items-end gap-3"
            onSubmit={(event) => {
              event.preventDefault();
              search();
            }}
          >
            {config.filters.map((filter) => (
              <FilterField
                key={filter.key}
                filter={filter}
                value={draft[filter.key] ?? ""}
                onChange={(value) => setDraft((d) => ({ ...d, [filter.key]: value }))}
              />
            ))}
            <div className="flex gap-2">
              <Button type="submit" size="sm">
                <Search /> {commonText("common.search")}</Button>
              <Button type="button" size="sm" variant="outline" onClick={clear}>
                <X /> {commonText("common.clear")}</Button>
            </div>
          </AutoForm>
        )}

        {!hideToolbar && <div className="ml-auto flex flex-wrap items-center gap-2">
          {/* 刷新 is a split control: the left half acts, the right half picks
              the interval. */}
          <div className="flex items-center">
            <Button
              size="sm"
              variant="outline"
              className="rounded-r-none border-r-0"
              onClick={reload}
              disabled={loading}
            >
              <RefreshCw className={cn(loading && "animate-spin")} />
              {commonText("common.refresh")}</Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  size="sm"
                  variant="outline"
                  className="rounded-l-none px-2"
                  aria-label={commonText("table.autoRefresh")}
                >
                  {refresh ? `${refresh}s` : commonText("table.off")}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start">
                {(config.toolbar?.refresh ?? REFRESH_INTERVALS).map((seconds) => (
                  <DropdownMenuItem
                    key={seconds}
                    onClick={() => {
                      setRefresh(seconds);
                      try {
                        localStorage.setItem(REFRESH_KEY(config.resource), String(seconds));
                      } catch {
                        /* a private window refuses to store the preference */
                      }
                    }}
                  >
                    {seconds === 0 ? commonText("table.disableRefresh") : commonText("table.refreshEvery", { seconds })}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <span className="flex-1" />
          {config.grouping && <div role="group" aria-label={commonText("table.viewMode")} title={commonText("table.groupBy", { name: configText(config.grouping.label, config.grouping.labelKey) })} className="inline-flex rounded-md border p-0.5">
            {([["list", commonText("table.list"), List], ["groups", commonText("table.groups"), FolderTree]] as const).map(([value, label, Icon]) => <Button key={value} size="sm" variant={tableView === value ? "secondary" : "ghost"} aria-pressed={tableView === value} onClick={() => { setTableView(value); setPage(1); }}><Icon className="size-4" />{label}</Button>)}
          </div>}

          <ResourceTransfer key={`${workspace}/${config.resource}`} workspace={workspace} resource={config.resource} params={params}
            columns={config.columns.filter(column => !hiddenColumns.has(column.key)).map(column => ({ key: column.key, label: configText(column.label, column.labelKey) }))}
            canImport={config.toolbar?.import === true && permissions.has(`${config.permissionResource ?? config.resource}.create`)}
            canExport={config.toolbar?.export === true} onChanged={() => { void load(); }} />
          {config.toolbar?.content && (!config.toolbar.content.permission || permissions.has(config.toolbar.content.permission)) && config.toolbar.content.render(reload)}
          {canCreate && (
            <Button
              size="sm"
              onClick={() => navigate({ ...route, mode: "new" }, {})}
              asChild={false}
            >
              <Plus />
              {typeof config.toolbar?.create === "object" ? configText(config.toolbar.create.label, config.toolbar.create.labelKey) : commonText("common.create")}
            </Button>
          )}
          {!!config.columns.length && <DropdownMenu><DropdownMenuTrigger asChild><Button size="sm" variant="outline"><Columns3 />{commonText("table.columns")}</Button></DropdownMenuTrigger>
            <DropdownMenuContent align="end">{config.columns.map(column => <DropdownMenuCheckboxItem key={column.key} checked={!hiddenColumns.has(column.key)} disabled={hiddenColumns.size === config.columns.length - 1 && !hiddenColumns.has(column.key)} onSelect={event => event.preventDefault()} onCheckedChange={() => setHiddenColumns(current => { const next = new Set(current); if (next.has(column.key)) next.delete(column.key); else next.add(column.key); return next; })}>{configText(column.label, column.labelKey)}</DropdownMenuCheckboxItem>)}</DropdownMenuContent>
          </DropdownMenu>}
        </div>}
        </div>
        {data && !loading && !error && config.summary?.(data)}
      </div>

      {grouped ? <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-hidden">
        <nav aria-label={commonText("repository.path")} className="flex shrink-0 flex-wrap items-center gap-1 rounded-md border px-2 py-1">
          <Button size="icon" variant="ghost" aria-label={commonText("repository.parent")} title={commonText("repository.parent")} disabled={!folder} onClick={() => enterFolder(folder.split('/').slice(0, -1).join('/'))}><ArrowUp className="size-4" /></Button>
          <Button size="sm" variant="ghost" onClick={() => enterFolder("")}>{commonText("repository.all")}</Button>
          {folder.split('/').filter(Boolean).map((name, index, parts) => <span key={index} className="inline-flex min-w-0 items-center"><ChevronRight className="size-3 shrink-0 text-muted-foreground" /><Button size="sm" variant="ghost" className="max-w-48 truncate" title={name} aria-current={index === parts.length - 1 ? "page" : undefined} onClick={() => enterFolder(parts.slice(0, index + 1).join('/'))}>{name}</Button></span>)}
        </nav>
        <AutoTable<(typeof directoryRows)[number]> fill layout="auto" rowKey={entry => entry.path}
          state={{ rows: directoryRows.slice((page - 1) * pageSize, page * pageSize), total: directoryRows.length, query: { page, pageSize }, loading, error: error || undefined }}
          columns={[
            { field: 'name', name: commonText("common.name"), render: (_value, entry) => entry.row ? <span className="inline-flex items-center gap-2"><GitBranch className="size-4 shrink-0 text-muted-foreground" />{entry.name}</span> : <button type="button" className="inline-flex items-center gap-2 text-left hover:text-primary" onClick={() => enterFolder(entry.path)}><Folder className="size-4 shrink-0 text-primary" />{entry.name}</button> },
            { field: 'count', name: commonText("common.content"), render: (_value, entry) => entry.row ? commonText("dashboard.repositories") : commonText("repository.count", { count: entry.count }) },
            ...config.columns.filter(column => column.key !== 'full_name').map(column => ({ field: column.key, name: configText(column.label, column.labelKey), render: (_value: unknown, entry: (typeof directoryRows)[number]) => entry.row ? column.render ? column.render(entry.row) : String((entry.row as Record<string, unknown>)[column.key] ?? '') : '—' })),
          ]}
          sticky={{ header: true }} rowClickable={entry => !entry.row} onRowClick={entry => { if (!entry.row) enterFolder(entry.path); }}
          onPageChange={setPage} onPageSizeChange={value => { setPageSize(value); setPage(1); }} pageSizeOptions={[...PAGE_SIZES]} onRetry={reload} empty={commonText("repository.emptyGroup")} />
      </div> : <AutoTable<T> fill layout="auto" scrollRef={scrollRef} rowKey={config.rowKey}
        hiddenFields={hiddenColumns}
        columns={config.columns.map(column => ({ field: column.key, name: configText(column.label, column.labelKey), width: column.width ? `${column.width}px` : undefined, align: column.align, sortable: column.sortable, render: (_value, row) => column.render ? column.render(row) : String((row as Record<string, unknown>)[column.key] ?? "") }))}
        state={{ rows, total, query: { page, pageSize, sort: sort ? { field: sort.replace(/^-/, ""), direction: sort.startsWith("-") ? "desc" : "asc" } : undefined }, loading, error: rows.length ? undefined : error, selected }}
        showSequence selectable={!!visibleBulk.length} pagination={config.pagination} hasMore={data?.has_more}
        sticky={{ header: true, left: 1, right: visibleActions.length ? 1 : 0, pagination: true }}
        groupBy={grouped ? config.grouping!.key : undefined} empty={config.emptyKey ? commonText(config.emptyKey) : config.empty}
        onPageChange={setPage}
        onPageSizeChange={value => { setPageSize(value); setPage(1); }}
        pageSizeOptions={[...PAGE_SIZES]} onSortChange={key => { setSort(sort === key ? `-${key}` : sort === `-${key}` ? "" : key); setPage(1); }}
        onRetry={reload} onSelectAll={toggleAll}
        onToggleSelect={row => setSelected(current => { const next = new Set(current); const key = config.rowKey(row); if (next.has(key)) next.delete(key); else next.add(key); return next; })}
        rowClickable={row => visibleActions.some(action => (action.label === "查看" || action.label === "编辑") && !!action.to && (!action.when || action.when(row)))}
        onRowClick={row => { const action = visibleActions.find(action => action.label === "查看" && action.to && (!action.when || action.when(row))); if (action?.to) navigate({ ...route, ...action.to(row) }, action.query?.(row) ?? {}); else navigate({ ...route, id: config.rowKey(row), mode: "view" }, {}); }}
        actionWidth={Math.max(80, visibleActions.length * 36 + 24)}
        actionsContent={visibleActions.length ? row => { const key = config.rowKey(row); return <>                          {visibleActions
                            .filter((action) => !action.when || action.when(row))
                            .map((action) => (
                              <RowActionButton
                                key={configText(action.label, action.labelKey)}
                                label={configText(action.label, action.labelKey)}
                                icon={busyActions.has(JSON.stringify([config.resource, key, action.label])) ? <Loader2 className="size-4 animate-spin" /> : action.icon ?? actionIcon(action.label)}
                                newTab={action.newTab}
                                disabled={busy || busyActions.has(JSON.stringify([config.resource, key, action.label])) || action.disabled?.(row)}
                                loading={busyActions.has(JSON.stringify([config.resource, key, action.label]))}
                                disabledReason={action.disabledReason?.(row)}
                                destructive={action.destructive}
                                {...(action.to
                                  ? { to: href({ ...route, ...action.to(row) }, action.query?.(row) ?? {}) }
                                  : {})}
                                {...(action.to
                                  ? { onNavigate: () => navigate({ ...route, ...action.to!(row) }, action.query?.(row) ?? {}) }
                                  : {})}
                                {...(action.action
                                  ? {
                                      onRun: async () => {
                                        if (action.destructive) {
                                          setPendingDelete({ action: action.label, actionKey: action.labelKey, target: rowName(row, config.rowKey(row)), run: async () => {
                                            if (action.parameters?.(row).length) { setPendingAction({ action, row }); return; }
                                            await action.action!(row);
                                            reload();
                                          } });
                                          return;
                                        }
                                        if(action.parameters?.(row).length){setPendingAction({action,row});return;}
                                        const actionKey = JSON.stringify([config.resource, key, action.label]);
                                        if (runningActions.current.has(actionKey)) return;
                                        runningActions.current.add(actionKey);
                                        setBusyActions(new Set(runningActions.current));
                                        try {
                                          await action.action!(row);
                                          reload();
                                        } catch (cause) {
                                          setError(`${rowName(row, key)}：${errorText(cause)}`);
                                        } finally {
                                          runningActions.current.delete(actionKey);
                                          setBusyActions(new Set(runningActions.current));
                                        }
                                      },
                                    }
                                  : {})}
                              />
                            ))}</>; } : undefined}
        footerContent={<>        {!!selected.size && (
          <>
            <span className="text-muted-foreground">{commonText("table.selected", { count: selected.size })}</span>
            {visibleBulk.map((action) => (
              <Button
                key={configText(action.label, action.labelKey)}
                size="sm"
                variant={action.destructive ? "destructive" : "outline"}
                disabled={busy}
                onClick={() => {
                  const ids = [...selected];
                  if (action.destructive) {
                    const names = ids.map(id => {
                      const row = rows.find(row => config.rowKey(row) === id);
                      return row ? rowName(row, id) : id;
                    });
                    setPendingDelete({ action: action.label, actionKey: action.labelKey, target: names.length <= 5 ? names.join("、") : commonText("table.selectionNames", { names: names.slice(0, 5).join(", "), count: ids.length }), run: async () => {
                      await action.action(ids);
                      setSelected(new Set());
                      reload();
                    } });
                  } else void runBulk(action.action, ids);
                }}
              >
                {busy && <Loader2 className="animate-spin" />}
                {configText(action.label, action.labelKey)}
              </Button>
            ))}
          </>
        )}
</>}
      />}
    </div>
  );
}

function RowActionButton({
  label,
  icon,
  newTab,
  destructive,
  to,
  onNavigate,
  onRun,
  disabled,
  disabledReason,
  loading,
}: {
  label: string;
  icon?: ReactNode;
  newTab?: boolean | undefined;
  disabled?: boolean | undefined;
  disabledReason?: string | undefined;
  loading?: boolean | undefined;
  destructive?: boolean | undefined;
  to?: string | undefined;
  onNavigate?: (() => void) | undefined;
  onRun?: (() => void | Promise<void>) | undefined;
}) {
  useCommonTranslation();
  // A navigating action stays a real link: middle-click, copy-link and
  // open-in-new-tab all keep working.
  if (to && !disabled)
    return (
      <a
        href={to}
        {...(newTab ? { target: "_blank", rel: "noopener noreferrer" } : {})}
        aria-label={icon ? label : undefined}
        title={label}
        className={cn(
          "mr-2 text-sm underline-offset-4 last:mr-0 hover:underline disabled:cursor-not-allowed disabled:opacity-50",
          icon && "inline-flex size-7 items-center justify-center align-middle",
          destructive ? "text-destructive" : "text-primary",
        )}
        onClick={(event) => {
          if (newTab) return;
          if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return;
          event.preventDefault();
          onNavigate?.();
        }}
      >
        {icon ?? label}
      </a>
    );
  return (
    <button
      type="button"
      className={cn(
        "mr-2 text-sm underline-offset-4 last:mr-0 hover:underline disabled:cursor-not-allowed disabled:opacity-50",
        icon && "inline-flex size-7 items-center justify-center align-middle",
        destructive ? "text-destructive" : "text-primary",
      )}
      disabled={disabled}
      aria-busy={loading || undefined}
      aria-label={label}
      title={disabled && disabledReason ? `${label}：${disabledReason}` : label}
      onClick={() => void onRun?.()}
    >
      {icon ?? label}
    </button>
  );
}

export function FilterField({
  filter,
  value,
  onChange,
}: {
  filter: NonNullable<ListConfig<unknown>["filters"]>[number];
  value: string;
  onChange: (value: string) => void;
}) {
  useCommonTranslation();
  const { options, error } = useOptions(filter.options);
  return (
    <AutoField label={configText(filter.label, filter.labelKey)}>
      {error && <span role="alert" className="text-xs text-destructive">{error}</span>}
      {filter.type === "select" ? (
        <Select value={value || "__all__"} onValueChange={(v) => onChange(v === "__all__" ? "" : v)}>
          <SelectTrigger aria-label={configText(filter.label, filter.labelKey)} size="sm" className="w-40">
            <SelectValue placeholder={commonText("group.all")} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">{commonText("group.all")}</SelectItem>
            {options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {configText(option.label, option.labelKey)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : (
        <Input
          type={filter.type}
          className="h-8 w-48"
          value={value}
          placeholder={configText(filter.placeholder, filter.placeholderKey)}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    </AutoField>
  );
}

export function readFilters(keys: string[], prefix?: string): Record<string, string> {
  const params = new URLSearchParams(location.search);
  return Object.fromEntries(keys.map((key) => [key, params.get(prefix ? `${prefix}.${key}` : key) ?? ""]));
}

/** Filters are replaceState so a search does not add a history entry per key. */
export function writeFilters(values: Record<string, string>, prefix?: string) {
  const params = new URLSearchParams(location.search);
  for (const [key, value] of Object.entries(values)) {
    const queryKey = prefix ? `${prefix}.${key}` : key;
    if (value) params.set(queryKey, value);
    else params.delete(queryKey);
  }
  params.delete("at");
  const search = params.toString();
  history.replaceState(history.state, "", `${location.pathname}${search ? `?${search}` : ""}`);
}
