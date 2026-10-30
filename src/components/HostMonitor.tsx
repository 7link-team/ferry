import { useEffect, useState } from "react";
import { Activity, HardDrive, MemoryStick, Network, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api, errorText } from "../api";

export interface Monitor {
  cpu_percent: number;
  load1: number;
  memory_used: number;
  memory_total: number;
  disks: { name: string; mount: string; used: number; total: number }[];
  networks: { name: string; received: number; transmitted: number }[];
}

export function isMonitor(value: unknown): value is Monitor {
  if (!value || typeof value !== "object") return false;
  const row = value as Partial<Monitor>;
  return typeof row.cpu_percent === "number" && Number.isFinite(row.cpu_percent)
    && typeof row.load1 === "number" && Number.isFinite(row.load1)
    && typeof row.memory_used === "number" && typeof row.memory_total === "number"
    && Array.isArray(row.disks) && row.disks.every(disk => disk && typeof disk.name === "string" && typeof disk.mount === "string" && typeof disk.used === "number" && typeof disk.total === "number")
    && Array.isArray(row.networks) && row.networks.every(network => network && typeof network.name === "string" && typeof network.received === "number" && typeof network.transmitted === "number");
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
type Sample = { at: number; monitor: Monitor };
function LineChart({ label, values, secondary, ceiling }: { label: string; values: number[]; secondary?: number[]; ceiling?: number }) {
  const max = ceiling ?? Math.max(1, ...values, ...(secondary ?? [])) * 1.15;
  const points = (series: number[]) => series.length < 2
    ? `0,${76 - (series[0] ?? 0) / max * 72} 300,${76 - (series[0] ?? 0) / max * 72}`
    : series.map((value, index) => `${index * 300 / (series.length - 1)},${76 - Math.min(max, Math.max(0, value)) / max * 72}`).join(" ");
  return <svg role="img" aria-label={label} viewBox="0 0 300 80" preserveAspectRatio="none" className="h-20 w-full overflow-visible">
    <line x1="0" y1="76" x2="300" y2="76" stroke="var(--border)" />
    <line x1="0" y1="40" x2="300" y2="40" stroke="var(--border)" strokeDasharray="3 4" />
    <polyline points={points(values)} fill="none" stroke="var(--primary)" strokeWidth="2" vectorEffect="non-scaling-stroke" strokeLinejoin="round" />
    {secondary && <polyline points={points(secondary)} fill="none" stroke="var(--success)" strokeWidth="2" vectorEffect="non-scaling-stroke" strokeLinejoin="round" />}
  </svg>;
}
function networkRates(samples: Sample[], name: string, key: "received" | "transmitted") {
  return samples.map((sample, index) => {
    if (index === 0) return 0;
    const current = sample.monitor.networks.find(network => network.name === name)?.[key];
    const previous = samples[index - 1]!.monitor.networks.find(network => network.name === name)?.[key];
    const seconds = (sample.at - samples[index - 1]!.at) / 1000;
    return current === undefined || previous === undefined || seconds <= 0 ? 0 : Math.max(0, (current - previous) / seconds);
  });
}

export function HostMonitor({ workspace, hostId }: { workspace: string; hostId: string }) {
  const [monitor, setMonitor] = useState<Monitor>();
  const [samples, setSamples] = useState<Sample[]>([]);
  const [error, setError] = useState("");
  const [interval, setIntervalSeconds] = useState(30);
  const [revision, setRevision] = useState(0);
  const [updatedAt, setUpdatedAt] = useState(0);
  useEffect(() => {
    let live = true;
    let busy = false;
    const refresh = () => {
      if (busy) return;
      busy = true;
      void api<Monitor>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/metrics`)
      .then(value => {
        if (!isMonitor(value)) throw new Error("监控接口尚未提供有效数据，请更新后端服务。");
        if (live) { const at = Date.now(); setMonitor(value); setSamples(current => [...current, { at, monitor: value }].slice(-60)); setError(""); setUpdatedAt(at); }
      })
      .catch(cause => { if (live) { setMonitor(undefined); setError(errorText(cause)); } })
      .finally(() => { busy = false; });
    };
    refresh();
    const timer = interval ? setInterval(refresh, interval * 1000) : undefined;
    return () => { live = false; if (timer) clearInterval(timer); };
  }, [workspace, hostId, interval, revision]);
  return <section className="mx-auto w-full max-w-6xl space-y-6 pb-8" aria-label="主机监控">
    <div className="flex flex-wrap items-center gap-3 border-b pb-4">
      <div className="flex items-center gap-2"><Activity className="size-4 text-primary" /><h2 className="text-sm font-semibold">实时监控</h2></div>
      <span className="flex-1" />
      {updatedAt > 0 && <span className="text-xs tabular-nums text-muted-foreground">更新于 {new Date(updatedAt).toLocaleTimeString()}</span>}
      <label className="flex items-center gap-2 text-xs text-muted-foreground">自动刷新<select aria-label="自动刷新间隔" className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground" value={interval} onChange={event => setIntervalSeconds(Number(event.target.value))}><option value={0}>关闭</option><option value={5}>5 秒</option><option value={15}>15 秒</option><option value={30}>30 秒</option><option value={60}>1 分钟</option></select></label>
      <Button type="button" variant="ghost" size="icon" aria-label="刷新监控" title="刷新监控" onClick={() => setRevision(value => value + 1)}><RotateCw className="size-4" /></Button>
    </div>
    {error && <p role="alert" className="border-l-2 border-destructive bg-destructive/5 px-3 py-2 text-sm text-destructive">监控暂不可用：{error}</p>}
    {!monitor && !error && <p role="status" className="py-8 text-sm text-muted-foreground">正在读取监控数据…</p>}
    {monitor && <>
      <div className="grid gap-8 border-b pb-7 md:grid-cols-2">
        <div className="space-y-3"><div className="flex items-center gap-2 text-sm text-muted-foreground"><Activity className="size-4" />CPU 使用率</div><div className="text-3xl font-semibold tabular-nums">{monitor.cpu_percent.toFixed(1)}<span className="ml-1 text-base font-normal text-muted-foreground">%</span></div><LineChart label="CPU 使用率趋势" values={samples.map(sample => sample.monitor.cpu_percent)} ceiling={100} /><p className="text-xs tabular-nums text-muted-foreground">一分钟负载 {monitor.load1.toFixed(2)}</p></div>
        <div className="space-y-3"><div className="flex items-center gap-2 text-sm text-muted-foreground"><MemoryStick className="size-4" />内存使用率</div><div className="text-3xl font-semibold tabular-nums">{percent(monitor.memory_used, monitor.memory_total)}</div><LineChart label="内存使用率趋势" values={samples.map(sample => ratio(sample.monitor.memory_used, sample.monitor.memory_total))} ceiling={100} /><p className="text-xs tabular-nums text-muted-foreground">{bytes(monitor.memory_used)} 已使用 / {bytes(monitor.memory_total)} 总量</p></div>
      </div>
      <div className="grid gap-8 lg:grid-cols-2">
        <section className="min-w-0"><h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><HardDrive className="size-4 text-muted-foreground" />磁盘 <span className="text-xs font-normal tabular-nums text-muted-foreground">{monitor.disks.length}</span></h3>{monitor.disks.length ? monitor.disks.map(disk => <div key={`${disk.name}/${disk.mount}`} className="space-y-2 border-t py-3 text-sm"><div className="flex min-w-0 flex-wrap justify-between gap-x-3 gap-y-1"><span className="min-w-0 truncate font-medium" title={`${disk.name} · ${disk.mount}`}>{disk.mount} <span className="font-normal text-muted-foreground">{disk.name}</span></span><strong className="tabular-nums">{percent(disk.used, disk.total)}</strong></div><LineChart label={`${disk.mount} 磁盘使用率趋势`} values={samples.map(sample => { const item = sample.monitor.disks.find(item => item.mount === disk.mount); return item ? ratio(item.used, item.total) : 0; })} ceiling={100} /><p className="text-xs tabular-nums text-muted-foreground">{bytes(disk.used)} / {bytes(disk.total)}</p></div>) : <p className="text-sm text-muted-foreground">暂无磁盘数据</p>}</section>
        <section className="min-w-0"><h3 className="mb-3 flex items-center gap-2 text-sm font-semibold"><Network className="size-4 text-muted-foreground" />网卡 <span className="text-xs font-normal tabular-nums text-muted-foreground">{monitor.networks.length}</span></h3>{monitor.networks.length ? monitor.networks.map(network => { const received = networkRates(samples, network.name, "received"); const transmitted = networkRates(samples, network.name, "transmitted"); return <div key={network.name} className="space-y-2 border-t py-3 text-sm"><div className="flex min-w-0 flex-wrap items-center justify-between gap-2"><span className="truncate font-medium" title={network.name}>{network.name}</span><span className="text-xs tabular-nums text-muted-foreground">累计接收 {bytes(network.received)} · 发送 {bytes(network.transmitted)}</span></div><LineChart label={`${network.name} 接收与发送速率趋势`} values={received} secondary={transmitted} /><p className="text-xs tabular-nums text-muted-foreground"><span className="text-primary">接收 {bytes(received.at(-1) ?? 0)}/s</span><span className="mx-3 text-success">发送 {bytes(transmitted.at(-1) ?? 0)}/s</span></p></div>; }) : <p className="text-sm text-muted-foreground">暂无网卡数据</p>}</section>
      </div>
    </>}
  </section>;
}
