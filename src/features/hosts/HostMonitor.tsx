import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { Activity,HardDrive,MemoryStick,Network } from "lucide-react";
import { useCallback,useEffect,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { useAutoRefresh } from "../../shared/api/use-auto-refresh";
import { RefreshControl } from "../../shared/ui/refresh-control";
import { AutoField } from "../../shared/forms/auto-form";
import { hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import { Checkbox } from "../../shared/ui/checkbox";
import { MonitorTimeChart,type MonitorPoint } from "./MonitorTimeChart";

export interface Monitor {
  cpu_percent: number;
  load1: number;
  memory_used: number;
  memory_total: number;
  disks: { name: string; mount: string; used: number; total: number }[];
  networks: { name: string; received: number; transmitted: number }[];
}

const nonnegative = (value: unknown): value is number => typeof value === "number" && Number.isFinite(value) && value >= 0;

export function isMonitor<T>(value: T): value is T & Monitor {
  if (!value || typeof value !== "object") return false;
  const row = value as Partial<Monitor>;
  return typeof row.cpu_percent === "number" && Number.isFinite(row.cpu_percent)
    && typeof row.load1 === "number" && Number.isFinite(row.load1)
    && nonnegative(row.memory_used) && nonnegative(row.memory_total)
    && Array.isArray(row.disks) && row.disks.every(disk => disk && typeof disk.name === "string" && typeof disk.mount === "string" && nonnegative(disk.used) && nonnegative(disk.total))
    && Array.isArray(row.networks) && row.networks.every(network => network && typeof network.name === "string" && nonnegative(network.received) && nonnegative(network.transmitted));
}

const bytes = (value: number) => {
  if (!Number.isFinite(value)) return "—";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let index = 0;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
  return `${value.toFixed(index ? 1 : 0)} ${units[index]}`;
};
const percent = (used: number, total: number) => total > 0 ? `${(used / total * 100).toFixed(1)}%` : "—";
const ratio = (used: number, total: number) => total > 0 ? Math.max(0, Math.min(100, used / total * 100)) : 0;
export type MonitorSample = { sampled_at: number; monitor: Monitor };
type MonitorResponse = Partial<Monitor> & {
  sampled_at?: number; server_time?: number; samples?: MonitorSample[];
  status?: "online" | "offline" | "collecting"; error?: string;
  collection_interval_seconds?: number;
};
/**
 * Loopback, container, hypervisor, VPN and OS filter interfaces, by name, as
 * monitoring dashboards conventionally exclude them. Names are a heuristic (a
 * Hyper-V external vEthernet carries real traffic), so all stay one click away.
 */
const VIRTUAL_NETWORK = new RegExp([
  "^lo\\d*$", "^(docker|veth|br-|virbr|vnet|cni|flannel|cali|weave|kube|vxlan|genev|dummy|tun|tap|wg|tailscale|zt|vmnet|vboxnet|utun|awdl|llw|anpi|bridge|gif|stf|ap\\d)",
  "loopback", "^vethernet", "isatap", "teredo", "npcap", "pseudo-interface", "virtualbox", "vmware", "hyper-v", "wan miniport", "kernel debug", "wi-fi direct", "^(本地连接|local area connection)\\*", "filter-\\d{4}$", "-(wfp|qos)", "zerotier", "openvpn", "wireguard", "tap-windows",
].join("|"), "i");
export const isVirtualNetwork = (name: string) => VIRTUAL_NETWORK.test(name.trim());

export function networkRates(samples: MonitorSample[], name: string, key: "received" | "transmitted", step: number): MonitorPoint[] {
  return samples.map((sample, index) => {
    const current = sample.monitor.networks.find(network => network.name === name)?.[key];
    const previousSample = samples[index - 1];
    const previous = previousSample?.monitor.networks.find(network => network.name === name)?.[key];
    const elapsed = previousSample ? sample.sampled_at - previousSample.sampled_at : 0;
    return { at: sample.sampled_at, value: current === undefined || previous === undefined || elapsed <= 0 || elapsed > step * 1.5 || current < previous
      ? null : (current - previous) / (elapsed / 1000) };
  });
}

export function HostMonitor({ workspace, hostId }: { workspace: string; hostId: string }) {
  useCommonTranslation();
  const [monitor, setMonitor] = useState<Monitor>();
  const [samples, setSamples] = useState<MonitorSample[]>([]);
  const [error, setError] = useState("");
  const [interval, setIntervalSeconds] = useState(30);
  const [revision, setRevision] = useState(0);
  const [pending, setPending] = useState(false);
  const refreshMonitor = useCallback(() => setRevision(value => value + 1), []);
  const remaining = useAutoRefresh(interval, refreshMonitor, pending);
  const [updatedAt, setUpdatedAt] = useState(0);
  const [rangeSeconds, setRangeSeconds] = useState(3600);
  const [windowEnd, setWindowEnd] = useState(Date.now());
  const [collectionSeconds, setCollectionSeconds] = useState(5);
  const [collecting, setCollecting] = useState(false);
  const [allNetworks, setAllNetworks] = useState(false);
  const step = (interval || 30) * 1000;
  const chart = { start: windowEnd - rangeSeconds * 1000, end: windowEnd, step };
  const points = (value: (monitor: Monitor) => number | null) => samples.map(sample => ({ at: sample.sampled_at, value: value(sample.monitor) }));
  const percentage = (value: number) => `${value.toFixed(0)}%`;
  useEffect(() => { setMonitor(undefined); setSamples([]); setUpdatedAt(0); setError(""); }, [workspace, hostId]);
  useEffect(() => {
    let live = true;
    let request: AbortController | undefined;
    const refresh = async () => {
      if (request) return;
      const controller = new AbortController(); request = controller;
      setPending(true);
      const timeout = setTimeout(() => controller.abort(), 8_000);
      try {
        const value = await api<MonitorResponse>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/metrics?window_seconds=${rangeSeconds}&step_seconds=${interval || 30}`, { signal: controller.signal });
        if (!live) return;
        const at = value.sampled_at ?? Date.now();
        const end = value.server_time ?? Date.now();
        if (!Number.isFinite(at) || at <= 0 || !Number.isFinite(end) || end <= 0) throw new Error(hostRepositoryText("host.009"));
        if (value.samples !== undefined && (!Array.isArray(value.samples) || !value.samples.every(sample => Number.isFinite(sample?.sampled_at) && sample.sampled_at > 0 && isMonitor(sample.monitor)))) throw new Error(hostRepositoryText("host.010"));
        if (!isMonitor(value) && value.status !== "collecting" && value.status !== "offline") throw new Error(hostRepositoryText("host.011"));
        setWindowEnd(end); setCollecting(value.status === "collecting");
        if (value.collection_interval_seconds) setCollectionSeconds(value.collection_interval_seconds);
        if (isMonitor(value)) { setMonitor(value); setUpdatedAt(at); }
        else { setMonitor(undefined); setUpdatedAt(0); }
        setSamples(current => {
          const next = value.samples ?? (isMonitor(value) ? [...current.filter(sample => sample.sampled_at !== at), { sampled_at: at, monitor: value }] : current);
          return [...new Map(next.filter(sample => sample.sampled_at >= end - rangeSeconds * 1000 && sample.sampled_at <= end).map(sample => [sample.sampled_at, sample])).values()].sort((a, b) => a.sampled_at - b.sampled_at).slice(-721);
        });
        setError(value.status === "offline" ? (value.error || hostRepositoryText("host.012")) : "");
      } catch (cause) {
        if (live) setError(controller.signal.aborted ? hostRepositoryText("host.013") : errorText(cause));
      } finally { clearTimeout(timeout); request = undefined; if (live) setPending(false); }
    };
    void refresh();
    return () => { live = false; request?.abort(); };
  }, [workspace, hostId, interval, revision, rangeSeconds]);
  const physical = monitor?.networks.filter(network => !isVirtualNetwork(network.name)) ?? [];
  const virtualCount = (monitor?.networks.length ?? 0) - physical.length;
  // Never hide everything: a host whose only adapters look virtual shows them all.
  const networks = allNetworks || !physical.length ? monitor?.networks ?? [] : physical;
  return <section className="w-full min-w-0 space-y-6 pb-8" aria-label={hostRepositoryText("host.014")}>
    <div className="flex flex-wrap items-center gap-3 border-b pb-4">
      <div className="flex items-center gap-2"><Activity className="size-4 text-primary" /><h2 className="text-sm font-semibold">{hostRepositoryText("host.015")}</h2></div>
      <AutoField label={hostRepositoryText("host.016")} className="!flex-row shrink-0 items-center gap-2 whitespace-nowrap text-xs text-muted-foreground"><select aria-label={hostRepositoryText("host.017")} className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground" value={rangeSeconds} onChange={event => setRangeSeconds(Number(event.target.value))}><option value={300}>{hostRepositoryText("host.018")}</option><option value={900}>{hostRepositoryText("host.019")}</option><option value={3600}>{hostRepositoryText("host.020")}</option></select></AutoField>
      <span className="flex-1" />
      {updatedAt > 0 && <span className="text-xs tabular-nums text-muted-foreground">{hostRepositoryText("host.021")}{" "}{new Date(updatedAt).toLocaleTimeString(currentLocale())}</span>}
      <RefreshControl loading={pending} interval={interval} remaining={remaining} onRefresh={refreshMonitor} onIntervalChange={setIntervalSeconds} />

    </div>
    <p className="text-xs leading-5 tabular-nums text-muted-foreground">{new Date(chart.start).toLocaleTimeString(currentLocale(), { hour12: false })} — {new Date(chart.end).toLocaleTimeString(currentLocale(), { hour12: false })} · {hostRepositoryText("host.053", {collection: collectionSeconds, step: interval || 30, refresh: interval ? hostRepositoryText("host.052", {value1: interval}) : hostRepositoryText("host.030"), count: samples.length})}</p>
    {error && <p role="alert" className="border-l-2 border-destructive bg-destructive/5 px-3 py-2 text-sm text-destructive">{hostRepositoryText("host.031")}{" "}{error}</p>}
    {!monitor && !error && (collecting ? <p role="status" className="py-8 text-sm text-muted-foreground">{hostRepositoryText("host.032")}</p> : <div className="grid gap-8 md:grid-cols-2">{[0, 1].map(index => <LoadingSkeleton key={index} variant="editor" label={hostRepositoryText("host.033")} />)}</div>)}
    {monitor && <>
      <div className="grid gap-8 border-b pb-7 md:grid-cols-2">
        <div className="space-y-3"><div className="flex items-center gap-2 text-sm text-muted-foreground"><Activity className="size-4" />{hostRepositoryText("host.034")}</div><div className="text-3xl font-semibold tabular-nums">{monitor.cpu_percent.toFixed(1)}<span className="ml-1 text-base font-normal text-muted-foreground">%</span></div><MonitorTimeChart {...chart} label={hostRepositoryText("host.035")} series={[{ name: "CPU", points: points(value => value.cpu_percent) }]} ceiling={100} format={percentage} /><p className="text-xs tabular-nums text-muted-foreground">{hostRepositoryText("host.036")}{" "}{monitor.load1.toFixed(2)}</p></div>
        <div className="space-y-3"><div className="flex items-center gap-2 text-sm text-muted-foreground"><MemoryStick className="size-4" />{hostRepositoryText("host.037")}</div><div className="text-3xl font-semibold tabular-nums">{percent(monitor.memory_used, monitor.memory_total)}</div><MonitorTimeChart {...chart} label={hostRepositoryText("host.038")} series={[{ name: hostRepositoryText("host.039"), points: points(value => value.memory_total > 0 ? ratio(value.memory_used, value.memory_total) : null) }]} ceiling={100} format={percentage} /><p className="text-xs tabular-nums text-muted-foreground">{bytes(monitor.memory_used)} {hostRepositoryText("host.040")}{" "}{bytes(monitor.memory_total)} {hostRepositoryText("host.041")}</p></div>
      </div>
      <div className="grid gap-8 lg:grid-cols-2">
        <section className="min-w-0"><h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><HardDrive className="size-4 text-muted-foreground" />{hostRepositoryText("host.042")}{" "}<span className="text-xs font-normal tabular-nums text-muted-foreground">{monitor.disks.length}</span></h3>{monitor.disks.length ? monitor.disks.map(disk => <div key={`${disk.name}/${disk.mount}`} className="space-y-2 border-t py-3 text-sm"><div className="flex min-w-0 flex-wrap justify-between gap-x-3 gap-y-1"><span className="min-w-0 truncate font-medium" title={`${disk.name} · ${disk.mount}`}>{disk.mount} <span className="font-normal text-muted-foreground">{disk.name}</span></span><strong className="tabular-nums">{percent(disk.used, disk.total)}</strong></div><MonitorTimeChart {...chart} label={hostRepositoryText("host.049", {value1: disk.mount})} series={[{ name: hostRepositoryText("host.042"), points: points(value => { const item = value.disks.find(item => item.mount === disk.mount && item.name === disk.name); return item && item.total > 0 ? ratio(item.used, item.total) : null; }) }]} ceiling={100} format={percentage} /><p className="text-xs tabular-nums text-muted-foreground">{bytes(disk.used)} / {bytes(disk.total)}</p></div>) : <p className="text-sm text-muted-foreground">{hostRepositoryText("host.043")}</p>}</section>
        <section className="min-w-0"><div className="mb-3 flex flex-wrap items-center justify-between gap-2"><h3 className="flex items-center gap-2 text-sm font-semibold"><Network className="size-4 text-muted-foreground" />{hostRepositoryText("host.044")}{" "}<span className="text-xs font-normal tabular-nums text-muted-foreground">{networks.length}{networks.length !== monitor.networks.length && ` / ${monitor.networks.length}`}</span></h3>{virtualCount > 0 && <AutoField label={hostRepositoryText("host.050", {value1: virtualCount})} className="flex-row-reverse items-center justify-end gap-2 text-xs text-muted-foreground"><Checkbox checked={allNetworks} onCheckedChange={checked => setAllNetworks(checked === true)} /></AutoField>}</div>{networks.length ? networks.map(network => { const received = networkRates(samples, network.name, "received", step); const transmitted = networkRates(samples, network.name, "transmitted", step); return <div key={network.name} className="space-y-2 border-t py-3 text-sm"><div className="flex min-w-0 flex-wrap items-center justify-between gap-2"><span className="truncate font-medium" title={network.name}>{network.name}</span><span className="text-xs tabular-nums text-muted-foreground">{hostRepositoryText("host.048")}{" "}{bytes(network.received)} · {hostRepositoryText("host.047")} {bytes(network.transmitted)}</span></div><MonitorTimeChart {...chart} label={hostRepositoryText("host.051", {value1: network.name})} series={[{ name: hostRepositoryText("host.046"), points: received }, { name: hostRepositoryText("host.047"), points: transmitted, secondary: true }]} format={value => `${bytes(value)}/s`} /><p className="text-xs tabular-nums text-muted-foreground"><span className="text-primary">{hostRepositoryText("host.046")}{" "}{received.at(-1)?.value == null ? "—" : `${bytes(received.at(-1)!.value!)}/s`}</span><span className="mx-3 text-success">{hostRepositoryText("host.047")}{" "}{transmitted.at(-1)?.value == null ? "—" : `${bytes(transmitted.at(-1)!.value!)}/s`}</span></p></div>; }) : <p className="text-sm text-muted-foreground">{hostRepositoryText("host.045")}</p>}</section>
      </div>
    </>}
  </section>;
}
