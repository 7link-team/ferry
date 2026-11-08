import { AutoForm } from "../shared/forms/auto-form";
import { LoadingSkeleton } from "../shared/ui/loading-skeleton";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { RefreshCw } from "lucide-react";
import { api, errorText } from "../shared/api/api";
import { useAutoRefresh } from "../shared/api/use-auto-refresh";
import { commonText, useCommonTranslation } from "../shared/i18n/i18n";
import { AutoTable } from "../shared/tables/auto-table";
import { Button } from "../shared/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../shared/ui/tabs";
import { Input } from "../shared/ui/input";
import { useQueryValue, useScrollSync } from "./router";
import { PlatformFileCatalog } from "./PlatformFileCatalog";
import { DeleteConfirmDialog } from "../shared/ui/DeleteConfirmDialog";
import { ResourceTable } from "./resources/ResourceTable";
import type { ListConfig } from "../shared/resources/types";

type Row = Record<string, unknown>;
type Snapshot = Record<string, any>;
const t = (key: string) => commonText(`resource-domain:monitor.${key}`);
export const monitorSections = ["database", "redis", "files", "usage"] as const;
const bytes = (value: unknown) => {
  if (value == null) return "—";
  let number = Number(value), unit = 0;
  while (number >= 1024 && unit < 4) { number /= 1024; unit++; }
  return `${number.toLocaleString(undefined, { maximumFractionDigits: 1 })} ${["B", "KiB", "MiB", "GiB", "TiB"][unit]}`;
};
const text = (value: unknown): string => value == null ? "—" : typeof value === "object" ? JSON.stringify(value) : String(value);
const databaseTables: ListConfig<Row> = {
  resource:"database-tables", endpoint:"/platform/monitor/database/tables", queryPrefix:"db_tables",
  rowKey:row=>String(row.name), toolbar:{create:false,import:false,export:false},
  filters:[
    {key:"q",label:"Table name",labelKey:"resource-domain:monitor.name",type:"text"},
    {key:"kind",label:"Type",labelKey:"resource-domain:monitor.kind",type:"select",options:[
      {value:"table",label:"Table",labelKey:"resource-domain:monitor.tableKind"},
      {value:"view",label:"View",labelKey:"resource-domain:monitor.viewKind"},
    ]},
  ],
  columns:[
    {key:"name",label:"Name",labelKey:"resource-domain:monitor.name",sortable:true,render:row=><span className="block max-w-80 whitespace-normal break-all">{String(row.name)}</span>},
    {key:"kind",label:"Type",labelKey:"resource-domain:monitor.kind",sortable:true,render:row=>t(row.kind==="view"?"viewKind":"tableKind")},
    {key:"estimated_rows",label:"Estimated rows",labelKey:"resource-domain:monitor.estimated_rows",sortable:true,align:"right",render:row=>row.estimated_rows==null?t("notCollected"):Number(row.estimated_rows).toLocaleString()},
    ...["bytes","data_bytes","index_bytes"].map(key=>({key,label:key,labelKey:`resource-domain:monitor.${key}`,sortable:true,align:"right" as const,
      render:(row:Row)=>row[key]==null?t(row.size_status==="not_applicable"?"sizeNotApplicable":"sizeUnavailable"):bytes(row[key])})),
  ],
};
function Metric({ label, value }: { label: string; value: ReactNode }) {
  return <div className="min-w-0 rounded-lg border bg-card p-4"><p className="text-sm text-muted-foreground">{label}</p><p className="mt-2 break-words text-2xl font-semibold tabular-nums">{value}</p></div>;
}
function Data({ title, rows, columns, sql=false }: { title: string; rows: Row[]; columns: string[]; sql?:boolean }) {
  return <section className="min-w-0 space-y-3"><h3 className="font-medium">{title}</h3><div className="min-w-0 rounded-lg border"><AutoTable data={rows} columns={columns.map(field => ({ field, name: t(field), sortable:sql, render: value => <span className={`block max-w-xl whitespace-normal break-words ${sql&&((field==="slow"&&Number(value)>0)||(["avg_ms","max_ms"].includes(field)&&Number(value)>=1000))?"font-semibold text-destructive":""}`}>{field==="avg_ms"?Number(value).toFixed(2):text(value)}{sql&&field==="slow"&&Number(value)>0&&<span className="ml-1 text-xs">{t("slowFlag")}</span>}</span> }))} rowKey={row => sql?String(row.template):String(rows.indexOf(row))} pageSize={10} /></div></section>;
}
function Trend({ rows, field, label }: { rows: Row[]; field: string; label: string }) {
  const max = Math.max(1, ...rows.map(row => Number(row[field] ?? 0)));
  const points = rows.map((row, i) => `${40 + i * 720 / Math.max(1, rows.length - 1)},${170 - Number(row[field] ?? 0) / max * 140}`).join(" ");
  return <section className="min-w-0 rounded-lg border bg-card p-4"><h3 className="font-medium">{label}</h3>{rows.length ? <svg role="img" aria-label={label} viewBox="0 0 800 210" className="mt-3 h-52 w-full"><title>{label}</title><path d="M40 20V170H770" fill="none" stroke="currentColor" className="text-border" /><polyline points={points} fill="none" stroke="currentColor" strokeWidth="2.5" className="text-primary" />{rows.map((row, i) => <circle key={i} cx={40 + i * 720 / Math.max(1, rows.length - 1)} cy={170 - Number(row[field] ?? 0) / max * 140} r="3" className="fill-primary"><title>{`${row.label ?? row.event_day}: ${row[field]}`}</title></circle>)}<text x="40" y="198" className="fill-muted-foreground text-xs">{text(rows[0]?.label)}</text><text x="760" y="198" textAnchor="end" className="fill-muted-foreground text-xs">{text(rows.at(-1)?.label)}</text><text x="36" y="20" textAnchor="end" className="fill-muted-foreground text-xs">{max.toLocaleString()}</text></svg> : <p className="py-12 text-center text-sm text-muted-foreground">{t("noSamples")}</p>}</section>;
}

export function PlatformMonitor({ section }: { section: string }) {
  useCommonTranslation();
  const active = monitorSections.includes(section as typeof monitorSections[number]) ? section : "database";
  const [period, setPeriod] = useQueryValue("period", "month");
  const [refresh, setRefresh] = useState(0);
  const [tab, setTab] = useQueryValue("tab", "overview");
  const [pending, setPending] = useState(false);
  const [data, setData] = useState<Snapshot>();
  const [error, setError] = useState("");
  const [resetSql, setResetSql] = useState(false);
  const [samples, setSamples] = useState<Row[]>([]);
  const [appliedPattern, setAppliedPattern] = useQueryValue("pattern", "*");
  const [cursor, setCursor] = useQueryValue("cursor", "0");
  const [pattern, setPattern] = useState(appliedPattern);
  useEffect(()=>setPattern(appliedPattern),[appliedPattern]);
  const [table, setTable] = useState("");
  const [tableRows, setTableRows] = useState<{ items: Row[]; columns: string[] }>();
  const sequence = useRef(0);
  const load = useCallback(async () => {
    const current = ++sequence.current;
    setPending(true);
    try {
      const query = new URLSearchParams({ period, pattern:appliedPattern, cursor });
      const value = await api<Snapshot>(`/platform/monitor/${active}?${query}`);
      if (current !== sequence.current) return;
      setData(value); setError("");
      if (active === "database" || active === "redis") setSamples(rows => [...rows.slice(-29), { label: new Date().toLocaleTimeString(), value: active === "database" ? value.pool?.active ?? 0 : Number(value.info?.used_memory ?? 0) }]);
    } catch (cause) { if (current === sequence.current) setError(errorText(cause)); }
    finally { if (current === sequence.current) setPending(false); }
  }, [active, period, appliedPattern, cursor]);
  useEffect(() => { setData(undefined); setSamples([]); setTableRows(undefined); }, [active]);
  useEffect(() => { void load(); return () => { sequence.current++; }; }, [load]);
  const remaining = useAutoRefresh(active==="database"&&tab==="tables"?0:refresh, load, pending);
  useEffect(() => {
    const controller = new AbortController();
    setTableRows(undefined);
    if (table && active === "database") void api<{ items: Row[]; columns: string[] }>(`/platform/monitor/database/table?name=${encodeURIComponent(table)}`, { signal: controller.signal }).then(setTableRows).catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
    return () => controller.abort();
  }, [table, active]);
  const daily = (data?.daily ?? []).map((row: Row) => ({ ...row, label: new Date(Number(row.event_day) * 86400000).toISOString().slice(5, 10) }));
  const scrollRef = useScrollSync(!!data);
  return <div ref={scrollRef} className="min-h-0 min-w-0 flex-1 overflow-auto">
    {resetSql&&<DeleteConfirmDialog action={t("resetCurrentSql")} target={t("sql")} description={t("resetSqlHelp")} onClose={()=>setResetSql(false)} onConfirm={async()=>{sequence.current++;await api('/platform/monitor/database/sql/reset',{method:'POST',body:JSON.stringify({instance_id:data?.instance_id})});await load();}}/>}
    <div className="space-y-5 pb-4">
      <div className="flex flex-wrap items-center gap-3">
        <h3 className="mr-auto font-semibold">{t(active)}</h3>
        {["files", "usage"].includes(active) && <select className="h-9 rounded-md border bg-background px-3 text-sm" aria-label={t("period")} value={period} onChange={event => setPeriod(event.target.value)}>{["day", "week", "month", "quarter", "year"].map(key => <option key={key} value={key}>{t(key)}</option>)}</select>}
        {!(active==="database"&&tab==="tables")&&<><Button variant="outline" size="sm" disabled={pending} onClick={() => void load()}><RefreshCw className={`size-4 ${pending ? "animate-spin" : ""}`} />{commonText("common.refresh")}</Button>
        <select className="h-9 rounded-md border bg-background px-3 text-sm" aria-label={commonText("table.autoRefresh")} value={refresh} onChange={event => setRefresh(Number(event.target.value))}>{[0, 5, 10, 15, 30, 60].map(seconds => <option key={seconds} value={seconds}>{seconds ? `${seconds}s` : commonText("table.off")}</option>)}</select>
        {!!refresh && <span role="timer" className="text-sm tabular-nums">{remaining}s</span>}</>}
      </div>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      {!data && pending && <LoadingSkeleton variant="page" />}
      {data && active === "database" && <Tabs value={tab} onValueChange={setTab} className="min-w-0">
        <TabsList aria-label={t(active)} className="max-w-full flex-wrap group-data-[orientation=horizontal]/tabs:h-auto"><TabsTrigger value="overview">{t("overview")}</TabsTrigger><TabsTrigger value="pools">{t("nodePools")}</TabsTrigger><TabsTrigger value="sql">{t("sql")}</TabsTrigger><TabsTrigger value="tables">{t("tables")}</TabsTrigger><TabsTrigger value="inspect">{t("inspect")}</TabsTrigger></TabsList>
        <TabsContent value="overview" className="space-y-5 pt-3">
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label={t("backend")} value={data.backend} /><Metric label={t("connections")} value={`${data.pool?.active ?? 0} / ${data.pool?.max ?? 0}`} /><Metric label={t("idle")} value={data.pool?.idle ?? 0} /><Metric label={t("bytes")} value={bytes(data.sqlite_bytes ?? data.instance?.bytes ?? (data.tables ?? []).reduce((sum: number, row: Row) => sum + Number(row.bytes ?? 0), 0))} /></div>
        <Data title={t("instance")} rows={Object.entries(data.instance ?? {}).map(([name, value]) => ({ name, value }))} columns={["name", "value"]} />
        <Trend rows={samples} field="value" label={t("poolTrend")} />
        </TabsContent><TabsContent value="pools" className="pt-3">
        <Data title={t("nodePools")} rows={(data.nodes ?? []).map((node: Snapshot) => ({ ...node, ...node.pool }))} columns={["instance_id", "online", "size", "active", "idle", "max"]} />
        </TabsContent><TabsContent value="sql" className="space-y-3 pt-3">
        <div className="flex flex-wrap items-center justify-between gap-3"><p className="break-all text-sm text-muted-foreground">{t("currentNodeSql")} · {data.instance_id}{data.sql_reset_at>0&&` · ${t("lastReset")} ${new Date(data.sql_reset_at).toLocaleString()}`}</p><Button variant="outline" size="sm" disabled={pending} onClick={()=>setResetSql(true)}>{t("resetCurrentSql")}</Button></div>
        <Data sql title={t("sql")} rows={(data.sql ?? []).map((row: Snapshot) => ({ ...row, avg_ms: row.count ? row.total_ms / row.count : 0 }))} columns={["template", "count", "errors", "slow", "total_ms", "avg_ms", "max_ms"]} />
        <p className="text-sm text-muted-foreground">{t("sqlHelp")}</p>
        <p className="text-sm text-muted-foreground">{t("sqlTimingHelp")}</p>
        <p className="text-sm text-muted-foreground">{t("sqlSkipped")}: {data.sql_skipped??0}</p>
        <p className="text-sm text-muted-foreground">{t("waitsHelp")}</p>
        {data.waits?.status==="available"?<Data title={t("waitsTitle")} rows={data.waits.items??[]} columns={["state","wait_type","wait_event","sessions","blocked","oldest_transaction_seconds"]}/>:<p role="status" className="text-sm text-muted-foreground">{t(data.waits?.status==="unsupported"?"waitsUnsupported":"waitsUnavailable")}</p>}
        </TabsContent><TabsContent value="tables" className="flex h-[65dvh] min-h-96 flex-col gap-3 pt-3">
        <p className="shrink-0 text-sm text-muted-foreground">{t("tableStatsHelp")}</p>
        <ResourceTable config={databaseTables} workspace="_platform" permissions={new Set()}/>
        </TabsContent><TabsContent value="inspect" className="space-y-5 pt-3">
        <label className="flex flex-wrap items-center gap-3 text-sm">{t("inspect")}<select className="h-9 max-w-full rounded-md border bg-background px-3" value={table} onChange={event => setTable(event.target.value)}><option value="">{commonText("common.select")}</option>{["worker_ids", "workspaces", "pipeline_runs", "workflow_executions", "repository_analysis_jobs", "platform_observations"].map(name => <option key={name}>{name}</option>)}</select></label>
        {tableRows && <Data title={`${table} · ${t("readonly")}`} rows={tableRows.items} columns={tableRows.columns} />}
        </TabsContent>
      </Tabs>}
      {data && active === "redis" && <Tabs value={tab} onValueChange={setTab} className="min-w-0">
          <TabsList aria-label={t(active)} className="max-w-full flex-wrap group-data-[orientation=horizontal]/tabs:h-auto"><TabsTrigger value="overview">{t("overview")}</TabsTrigger><TabsTrigger value="keys">{t("keys")}</TabsTrigger><TabsTrigger value="info">Redis INFO</TabsTrigger></TabsList>
          <TabsContent value="overview" className="space-y-5 pt-3">
          {!data.configured ? <p role="status">{t("redisDisabled")}</p> : <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label={t("connection")} value={data.connected ? t("connected") : t("disconnected")} /><Metric label={t("memory")} value={bytes(data.info?.used_memory)} /><Metric label={t("clients")} value={data.info?.connected_clients ?? "—"} /><Metric label={t("ops")} value={data.info?.instantaneous_ops_per_sec ?? "—"} /></div>
          <Data title={t("connection")} rows={Object.entries(data.connection ?? {}).map(([name, value]) => ({ name, value }))} columns={["name", "value"]} />
          {!data.connected && <p role="status">{t("redisUnavailable")}</p>}
          {data.connected && <>
            <Trend rows={samples} field="value" label={t("memoryTrend")} />
            <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label={t("hitRate")} value={(Number(data.info?.keyspace_hits ?? 0) + Number(data.info?.keyspace_misses ?? 0)) ? `${(100 * Number(data.info.keyspace_hits) / (Number(data.info.keyspace_hits) + Number(data.info.keyspace_misses))).toFixed(1)}%` : "—"} /><Metric label={t("slowlog")} value={data.slowlog_count ?? "—"} /><Metric label={t("evictions")} value={data.info?.evicted_keys ?? "—"} /><Metric label={t("version")} value={data.info?.redis_version ?? "—"} /></div>
          </>}
          </>}
          </TabsContent><TabsContent value="keys" className="space-y-5 pt-3">
            {!data.connected ? <p role="status">{t(data.configured ? "redisUnavailable" : "redisDisabled")}</p> : <>
            <AutoForm onSubmit={event=>{event.preventDefault();if(pattern===appliedPattern&&cursor==="0")void load();else {setAppliedPattern(pattern);setCursor("0");}}} className="flex flex-wrap items-center gap-2"><label className="text-sm" htmlFor="redis-pattern">{t("keyPattern")}</label><Input id="redis-pattern" className="w-56" value={pattern} maxLength={200} onChange={event => setPattern(event.target.value)} /><Button type="submit" variant="outline" size="sm" disabled={pending}>{commonText("common.search")}</Button><Button type="button" variant="outline" size="sm" disabled={pending || pattern!==appliedPattern || !data.cursor || String(data.cursor)==="0"} onClick={() => setCursor(String(data.cursor))}>{t("nextScan")}</Button><span role="status" className="text-sm text-muted-foreground">{t(pending?"scanPending":String(data.cursor)==="0"?"scanComplete":"scanMore")}</span></AutoForm>
            <Data title={t("keys")} rows={data.keys ?? []} columns={["key", "type", "ttl"]} />
            <p className="text-sm text-muted-foreground">{t("redisHelp")}</p>
            </>}
          </TabsContent><TabsContent value="info" className="pt-3">
            {!data.connected ? <p role="status">{t(data.configured ? "redisUnavailable" : "redisDisabled")}</p> : <Data title="Redis INFO" rows={Object.entries(data.info ?? {}).map(([name, value]) => ({ name, value }))} columns={["name", "value"]} />}
          </TabsContent>
      </Tabs>}
      {data && active === "files" && <Tabs value={tab} onValueChange={setTab} className="min-w-0">
        <TabsList aria-label={t(active)} className="max-w-full flex-wrap group-data-[orientation=horizontal]/tabs:h-auto"><TabsTrigger value="overview">{t("overview")}</TabsTrigger><TabsTrigger value="catalog">{t("fileCatalog")}</TabsTrigger><TabsTrigger value="top">{t("topFiles")}</TabsTrigger><TabsTrigger value="access">{t("fileAccess")}</TabsTrigger></TabsList>
        <TabsContent value="overview" className="space-y-5 pt-3">
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label={t("assets")} value={data.assets?.count ?? 0} /><Metric label={t("bytes")} value={bytes(data.assets?.bytes)} /><Metric label={t("artifacts")} value={data.artifacts?.count ?? 0} /><Metric label={t("artifactBytes")} value={bytes(data.artifacts?.bytes)} /></div>
        <Data title={t("fileTypes")} rows={data.types ?? []} columns={["mime", "count", "bytes"]} />
        <p className="text-sm text-muted-foreground">{t("filesHelp")}</p>
        </TabsContent><TabsContent value="catalog" className="pt-3"><PlatformFileCatalog/></TabsContent><TabsContent value="top" className="pt-3">
        <Data title={t("topFiles")} rows={data.top ?? []} columns={["file_path", "requests", "bytes", "last_at"]} />
        </TabsContent><TabsContent value="access" className="pt-3">
        <Data title={t("fileAccess")} rows={data.recent ?? []} columns={["at", "user_id", "file_path", "status", "response_bytes", "elapsed_ms"]} />
        </TabsContent>
      </Tabs>}
      {data && active === "usage" && <Tabs value={tab} onValueChange={setTab} className="min-w-0">
        <TabsList aria-label={t(active)} className="max-w-full flex-wrap group-data-[orientation=horizontal]/tabs:h-auto"><TabsTrigger value="overview">{t("overview")}</TabsTrigger><TabsTrigger value="daily">{t("daily")}</TabsTrigger><TabsTrigger value="routes">{t("topRoutes")}</TabsTrigger></TabsList>
        <TabsContent value="overview" className="space-y-5 pt-3">
        <p className="text-sm text-muted-foreground">{t("usageHelp")}</p>
        <p className="text-sm text-muted-foreground">{t("coverageHelp")} {t("retention")}: {data.retention_days} · {t("maxRows")}: {data.max_rows_per_node?.toLocaleString()} · {t("oldest")}: {data.oldest_at?new Date(data.oldest_at).toLocaleString():"—"}</p>
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">{["day", "week", "month", "quarter", "year"].map(period => <Metric key={period} label={`${t(period)} · ${t("active_users")}`} value={data.active?.[period] ?? 0} />)}</div>
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label={t("requests")} value={data.summary?.requests ?? 0} /><Metric label={t("page_views")} value={data.summary?.page_views ?? 0} /><Metric label={t("registered")} value={data.users?.registered ?? 0} /><Metric label={t("onlineUsers")} value={data.online?.users ?? 0} /></div>
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4"><Metric label={t("errors")} value={data.summary?.errors ?? 0} /><Metric label={t("avg_ms")} value={data.summary?.requests ? (Number(data.summary.elapsed_ms) / Number(data.summary.requests)).toFixed(1) : "—"} /><Metric label={t("max_ms")} value={data.summary?.max_ms ?? "—"} /><Metric label={t("dropped")} value={data.dropped_current_node ?? 0} /></div>
        </TabsContent><TabsContent value="daily" className="space-y-5 pt-3">
        <div className="grid gap-4 xl:grid-cols-2"><Trend rows={daily} field="active_users" label={t("active_users")} /><Trend rows={daily} field="requests" label={t("requests")} /></div>
        <Data title={t("daily")} rows={daily} columns={["label", "active_users", "requests", "page_views", "errors"]} />
        </TabsContent><TabsContent value="routes" className="pt-3">
        <Data title={t("topRoutes")} rows={data.routes ?? []} columns={["route", "kind", "requests", "errors", "max_ms"]} />
        </TabsContent>
      </Tabs>}
    </div>
  </div>;
}
