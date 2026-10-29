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
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ResourceActionDialog } from "./ResourceForm";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import type { RowAction } from "./dsl";
import {
  ChevronLeft,
  ChevronRight,
  Cable,
  Copy,
  Download,
  Ellipsis,
  Eye,
  FolderOpen,
  History,
  LayoutTemplate,
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
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "cn";
import { errorText } from "../api";
import {
  PAGE_SIZES,
  REFRESH_INTERVALS,
  listResource,
  pageRange,
  type Envelope,
} from "../resource";
import { href, navigate, useQueryValue, useRoute, useScrollSync } from "../router";
import type { ListConfig, Option } from "./dsl";
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
  useEffect(() => {
    if (typeof source !== "function") {
      setOptions(Array.isArray(source) ? source : []);
      return;
    }
    let live = true;
    void source().then((loaded) => live && setOptions(loaded));
    return () => {
      live = false;
    };
  }, [source]);
  return options;
}

export function ResourceTable<T>({
  config,
  workspace,
  permissions,
}: {
  config: ListConfig<T>;
  workspace: string;
  permissions: Set<string>;
}) {
  const route = useRoute();
  const [page, setPage] = useQueryValue("page", 1);
  const [pageSize, setPageSize] = useQueryValue("page_size", config.defaultPageSize ?? 20);
  const [sort, setSort] = useQueryValue("sort", "");

  // Two layers: `draft` is what the inputs show, the URL is what has been
  // searched. Typing must not refetch on every keystroke, and the URL must
  // stay shareable, so the committed values are the ones in the query string.
  const filterKeys = useMemo(() => (config.filters ?? []).map((f) => f.key), [config.filters]);
  const [committed, setCommitted] = useState<Record<string, string>>(() => readFilters(filterKeys));
  const [draft, setDraft] = useState<Record<string, string>>(committed);

  const [data, setData] = useState<Envelope<T>>();
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [refresh, setRefresh] = useState(() => readRefresh(config.resource, config.defaultRefresh));
  const [busy, setBusy] = useState(false);
  const [pendingAction,setPendingAction]=useState<{action:RowAction<T>;row:T}>();
  const [pendingDelete, setPendingDelete] = useState<{ action: string; target: string; run: () => Promise<void> }>();
  const reloadToken = useRef(0);

  const params = useMemo(
    () => ({
      ...config.fixed,
      ...Object.fromEntries(Object.entries(committed).filter(([, v]) => v !== "")),
      page,
      page_size: pageSize,
      ...(sort ? { sort } : {}),
    }),
    [config.fixed, committed, page, pageSize, sort],
  );

  const load = useCallback(
    (signal?: AbortSignal) => {
      setLoading(true);
      return listResource<T>(workspace, config.resource, params, signal, config.endpoint)
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
    [workspace, config.resource, config.endpoint, params],
  );

  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load, reloadToken.current]);

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
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const rows = data?.items ?? [];
  const scrollRef = useScrollSync(!loading && rows.length > 0);

  const search = () => {
    setCommitted(draft);
    writeFilters(draft);
    setPage(1);
    setSelected(new Set());
  };
  const clear = () => {
    const empty = Object.fromEntries(filterKeys.map((key) => [key, ""]));
    setDraft(empty);
    setCommitted(empty);
    writeFilters(empty);
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

  const columnCount = config.columns.length + 1 + (visibleActions.length ? 1 : 0);

  return (
    <div className="grid min-h-0 min-w-0 flex-1 grid-rows-[auto_minmax(0,1fr)_auto] gap-3">
      {/* ---------------------------------------------------- 搜索过滤 + 工具栏 */}
      {pendingAction&&<ResourceActionDialog action={pendingAction.action} row={pendingAction.row} workspace={workspace} permissions={permissions} onClose={()=>setPendingAction(undefined)} onDone={()=>{setPendingAction(undefined);reload();}} />}
      {pendingDelete && <DeleteConfirmDialog action={pendingDelete.action} target={pendingDelete.target} onClose={() => setPendingDelete(undefined)} onConfirm={pendingDelete.run} />}
      <div className="flex flex-col gap-3">
        {!!config.filters?.length && (
          <form
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
                <Search /> 搜索
              </Button>
              <Button type="button" size="sm" variant="outline" onClick={clear}>
                <X /> 清空
              </Button>
            </div>
          </form>
        )}

        <div className="flex flex-wrap items-center gap-2">
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
              刷新
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  size="sm"
                  variant="outline"
                  className="rounded-l-none px-2"
                  aria-label="定时刷新"
                >
                  {refresh ? `${refresh}s` : "关"}
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
                    {seconds === 0 ? "关闭自动刷新" : `每 ${seconds} 秒`}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          <span className="flex-1" />

          <ResourceTransfer key={`${workspace}/${config.resource}`} workspace={workspace} resource={config.resource} params={params}
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
              {typeof config.toolbar?.create === "object" ? config.toolbar.create.label : "创建"}
            </Button>
          )}
        </div>
      </div>

      {/* ------------------------------------------------------------ 列表 */}
      <div
        ref={scrollRef}
        className="min-h-0 min-w-0 overflow-auto rounded-lg border border-border bg-card"
      >
        <table className="w-full min-w-max border-separate border-spacing-0 text-sm">
          <thead>
            <tr>
              <th className="sticky top-0 left-0 z-40 w-14 bg-muted px-3 py-2 text-left font-medium text-muted-foreground shadow-[inset_-1px_0_0_var(--border),inset_0_-1px_0_var(--border)]">
                序号
              </th>
              {config.columns.map((column) => (
                <th
                  key={column.key}
                  style={column.width ? { width: column.width } : undefined}
                  className={cn(
                    "sticky top-0 z-30 bg-muted px-3 py-2 font-medium text-muted-foreground shadow-[inset_0_-1px_0_var(--border)]",
                    column.align === "right" ? "text-right" : "text-left",
                  )}
                >
                  {column.sortable ? (
                    <button
                      type="button"
                      className="hover:text-foreground"
                      onClick={() =>
                        setSort(
                          sort === column.key
                            ? `-${column.key}`
                            : sort === `-${column.key}`
                              ? ""
                              : column.key,
                        )
                      }
                    >
                      {column.label}
                      {sort === column.key ? " ↑" : sort === `-${column.key}` ? " ↓" : ""}
                    </button>
                  ) : (
                    column.label
                  )}
                </th>
              ))}
              {!!visibleActions.length && (
                <th className="sticky top-0 right-0 z-40 w-px bg-muted px-3 py-2 text-right font-medium text-muted-foreground shadow-[inset_1px_0_0_var(--border),inset_0_-1px_0_var(--border)]">
                  操作
                </th>
              )}
            </tr>
          </thead>
          <tbody>
            {loading && !data
              ? Array.from({ length: 5 }, (_, index) => (
                  <tr key={index}>
                    {Array.from({ length: columnCount }, (_, cell) => (
                      <td key={cell} className="border-b border-border px-3 py-2">
                        <Skeleton className="h-4 w-full" />
                      </td>
                    ))}
                  </tr>
                ))
              : rows.map((row, index) => {
                  const key = config.rowKey(row);
                  const view = visibleActions.find(action => action.label === "查看" && action.to && (!action.when || action.when(row)));
                  const canOpen = !!view || visibleActions.some(action => action.label === "编辑" && action.to && (!action.when || action.when(row)));
                  const open = () => { if (view?.to) navigate({ ...route, ...view.to(row) }, view.query?.(row) ?? {}); else if (canOpen) navigate({ ...route, id: key, mode: "view" }, {}); };
                  return (
                    <tr key={key} className={cn("group", canOpen && "cursor-pointer focus-visible:outline-2 focus-visible:outline-primary")} tabIndex={canOpen ? 0 : undefined}
                      onClick={event => { if (!(event.target as Element).closest("a,button,input,select,textarea,[role=button],[role=checkbox]")) open(); }}
                      onKeyDown={event => { if (canOpen && event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) { event.preventDefault(); open(); } }}>
                      <td className="sticky left-0 z-20 bg-card px-3 py-2 text-muted-foreground shadow-[inset_-1px_0_0_var(--border)] group-hover:bg-accent">
                        <span className="inline-flex items-center gap-2">
                          {!!visibleBulk.length && (
                            <Checkbox
                              checked={selected.has(key)}
                              onCheckedChange={(checked) =>
                                setSelected((current) => {
                                  const next = new Set(current);
                                  if (checked) next.add(key);
                                  else next.delete(key);
                                  return next;
                                })
                              }
                              aria-label="选择此行"
                            />
                          )}
                          {(page - 1) * pageSize + index + 1}
                        </span>
                      </td>
                      {config.columns.map((column) => (
                        <td
                          key={column.key}
                          className={cn(
                            "border-b border-border px-3 py-2 group-hover:bg-accent",
                            column.align === "right" && "text-right",
                          )}
                        >
                          {column.render
                            ? column.render(row)
                            : String((row as Record<string, unknown>)[column.key] ?? "")}
                        </td>
                      ))}
                      {!!visibleActions.length && (
                        <td className="sticky right-0 z-20 bg-card px-3 py-2 text-right whitespace-nowrap shadow-[inset_1px_0_0_var(--border)] group-hover:bg-accent">
                          {visibleActions
                            .filter((action) => !action.when || action.when(row))
                            .map((action) => (
                              <RowActionButton
                                key={action.label}
                                label={action.label}
                                icon={action.icon ?? actionIcon(action.label)}
                                newTab={action.newTab}
                                disabled={busy || action.disabled?.(row)}
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
                                          setPendingDelete({ action: action.label, target: rowName(row, config.rowKey(row)), run: async () => {
                                            if (action.parameters?.(row).length) { setPendingAction({ action, row }); return; }
                                            await action.action!(row);
                                            reload();
                                          } });
                                          return;
                                        }
                                        if(action.parameters?.(row).length){setPendingAction({action,row});return;}
                                        setBusy(true);
                                        try {
                                          await action.action!(row);
                                          reload();
                                        } catch (cause) {
                                          setError(errorText(cause));
                                        } finally {
                                          setBusy(false);
                                        }
                                      },
                                    }
                                  : {})}
                              />
                            ))}
                        </td>
                      )}
                    </tr>
                  );
                })}
            {!loading && !rows.length && (
              <tr>
                <td colSpan={columnCount} className="px-3 py-16 text-center text-muted-foreground">
                  {error ? (
                    <span className="text-destructive">{error}</span>
                  ) : (
                    (config.empty ?? "没有符合条件的记录。")
                  )}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* ------------------------------------------------------------ 底部 */}
      <div className="flex flex-wrap items-center gap-3 pb-3 text-sm">
        {!!visibleBulk.length && (
          <label className="flex items-center gap-2">
            <Checkbox checked={allOnPageSelected} onCheckedChange={toggleAll} />
            全选
          </label>
        )}
        {!!selected.size && (
          <>
            <span className="text-muted-foreground">已选 {selected.size} 项</span>
            {visibleBulk.map((action) => (
              <Button
                key={action.label}
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
                    setPendingDelete({ action: action.label, target: names.length <= 5 ? names.join("、") : `${names.slice(0, 5).join("、")}等 ${ids.length} 项`, run: async () => {
                      await action.action(ids);
                      setSelected(new Set());
                      reload();
                    } });
                  } else void runBulk(action.action, ids);
                }}
              >
                {busy && <Loader2 className="animate-spin" />}
                {action.label}
              </Button>
            ))}
          </>
        )}
        <span className="flex-1" />
        <span className="text-muted-foreground">共 {total} 条</span>
        <nav className="flex items-center gap-1" aria-label="分页">
          <Button
            size="icon"
            variant="outline"
            className="size-8"
            disabled={page <= 1}
            onClick={() => setPage(page - 1)}
            aria-label="上一页"
          >
            <ChevronLeft />
          </Button>
          {pageRange(page, pageCount).map((value, index) =>
            value === null ? (
              <span key={`gap-${index}`} className="px-1 text-muted-foreground">
                …
              </span>
            ) : (
              <Button
                key={value}
                size="icon"
                variant={value === page ? "default" : "outline"}
                className="size-8"
                onClick={() => setPage(value)}
                aria-current={value === page ? "page" : undefined}
              >
                {value}
              </Button>
            ),
          )}
          <Button
            size="icon"
            variant="outline"
            className="size-8"
            disabled={page >= pageCount}
            onClick={() => setPage(page + 1)}
            aria-label="下一页"
          >
            <ChevronRight />
          </Button>
        </nav>
        <Select
          value={String(pageSize)}
          onValueChange={(value) => {
            setPageSize(Number(value));
            setPage(1);
          }}
        >
          <SelectTrigger size="sm" className="w-28">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {Array.from(new Set([...PAGE_SIZES, pageSize])).sort((a,b) => a-b).map((size) => (
              <SelectItem key={size} value={String(size)}>
                {size} 条/页
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
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
}: {
  label: string;
  icon?: ReactNode;
  newTab?: boolean | undefined;
  disabled?: boolean | undefined;
  disabledReason?: string | undefined;
  destructive?: boolean | undefined;
  to?: string | undefined;
  onNavigate?: (() => void) | undefined;
  onRun?: (() => void | Promise<void>) | undefined;
}) {
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
      aria-label={label}
      title={disabled && disabledReason ? `${label}：${disabledReason}` : label}
      onClick={() => void onRun?.()}
    >
      {icon ?? label}
    </button>
  );
}

function FilterField({
  filter,
  value,
  onChange,
}: {
  filter: NonNullable<ListConfig<unknown>["filters"]>[number];
  value: string;
  onChange: (value: string) => void;
}) {
  const options = useOptions(filter.options);
  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{filter.label}</span>
      {filter.type === "select" ? (
        <Select value={value || "__all__"} onValueChange={(v) => onChange(v === "__all__" ? "" : v)}>
          <SelectTrigger aria-label={filter.label} size="sm" className="w-40">
            <SelectValue placeholder="全部" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all__">全部</SelectItem>
            {options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : (
        <Input
          className="h-8 w-48"
          value={value}
          placeholder={filter.placeholder}
          onChange={(event) => onChange(event.target.value)}
        />
      )}
    </label>
  );
}

function readFilters(keys: string[]): Record<string, string> {
  const params = new URLSearchParams(location.search);
  return Object.fromEntries(keys.map((key) => [key, params.get(key) ?? ""]));
}

/** Filters are replaceState so a search does not add a history entry per key. */
function writeFilters(values: Record<string, string>) {
  const params = new URLSearchParams(location.search);
  for (const [key, value] of Object.entries(values)) {
    if (value) params.set(key, value);
    else params.delete(key);
  }
  params.delete("at");
  const search = params.toString();
  history.replaceState(history.state, "", `${location.pathname}${search ? `?${search}` : ""}`);
}
