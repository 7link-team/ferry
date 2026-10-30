import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { Activity, HardDrive, MemoryStick } from "lucide-react";
import { api, errorText } from "../api";
import { isMonitor, type Monitor } from "./HostMonitor";

type Sample = { monitor?: Monitor; error?: string };
const Context = createContext<Sample | undefined>(undefined);

export function HostTelemetry({ workspace, hostId, enabled, children }: {
  workspace: string; hostId: string; enabled: boolean; children: ReactNode;
}) {
  const [sample, setSample] = useState<Sample>({});
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let request: AbortController | undefined;
    const poll = async () => {
      if (disposed || document.hidden || request) return;
      const controller = new AbortController(); request = controller;
      const timeout = setTimeout(() => controller.abort(), 8_000);
      try {
        const monitor = await api<unknown>(`/workspaces/${encodeURIComponent(workspace)}/hosts/${encodeURIComponent(hostId)}/monitor`, { signal: controller.signal });
        if (!isMonitor(monitor)) throw new Error("监控接口尚未提供有效数据");
        if (!disposed) setSample({ monitor });
      } catch (cause) {
        if (!disposed) setSample({ error: controller.signal.aborted ? "监控请求超时" : errorText(cause) });
      } finally {
        clearTimeout(timeout); request = undefined;
        if (!disposed) timer = setTimeout(() => { void poll(); }, 3_000);
      }
    };
    const resume = () => { if (!document.hidden) { clearTimeout(timer); void poll(); } };
    setSample({}); void poll();
    document.addEventListener("visibilitychange", resume);
    return () => {
      disposed = true; clearTimeout(timer); request?.abort();
      document.removeEventListener("visibilitychange", resume);
    };
  }, [workspace, hostId, enabled]);
  return <Context.Provider value={sample}>{children}</Context.Provider>;
}

const percent = (used: number, total: number) => total > 0 && Number.isFinite(used / total) ? `${Math.min(100, Math.max(0, used / total * 100)).toFixed(0)}%` : "—";
export function TerminalMetrics() {
  const sample = useContext(Context);
  if (!sample) return null;
  if (!sample.monitor) return <span className="text-xs text-muted-foreground" title={sample.error}>{sample.error ? "监控暂不可用" : "监控采集中…"}</span>;
  const monitor = sample.monitor;
  const diskUsed = monitor.disks.reduce((sum, disk) => sum + disk.used, 0);
  const diskTotal = monitor.disks.reduce((sum, disk) => sum + disk.total, 0);
  return <span aria-label="实时主机监控" className="inline-flex items-center gap-3 text-xs tabular-nums text-muted-foreground" title="每 3 秒刷新；所有终端分屏共用一次采集">
    <span className="inline-flex items-center gap-1" title="CPU 使用率"><Activity className="size-3.5" />CPU {monitor.cpu_percent.toFixed(0)}%</span>
    <span className="inline-flex items-center gap-1" title={`内存：${(monitor.memory_used / 1024 ** 3).toFixed(1)} / ${(monitor.memory_total / 1024 ** 3).toFixed(1)} GiB`}><MemoryStick className="size-3.5" />内存 {percent(monitor.memory_used, monitor.memory_total)}</span>
    <span className="inline-flex items-center gap-1" title={monitor.disks.map(disk => `${disk.mount || disk.name}：${percent(disk.used, disk.total)}`).join("\n")}><HardDrive className="size-3.5" />磁盘 {percent(diskUsed, diskTotal)}</span>
  </span>;
}
