import { hostRepositoryText, useCommonTranslation } from "../i18n";
import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { Activity, HardDrive, MemoryStick } from "lucide-react";
import { api, errorText } from "../api";
import { isMonitor, type Monitor } from "./HostMonitor";

type Sample = { monitor?: Monitor; error?: string };
const Context = createContext<Sample | undefined>(undefined);

export function HostTelemetry({ workspace, hostId, enabled, children }: {
  workspace: string; hostId: string; enabled: boolean; children: ReactNode;
}) {
  useCommonTranslation();
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
        const monitor = await api<unknown>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/metrics`, { signal: controller.signal });
        if (!isMonitor(monitor)) throw new Error(hostRepositoryText("host.054"));
        if (!disposed) setSample({ monitor });
      } catch (cause) {
        if (!disposed) setSample({ error: controller.signal.aborted ? hostRepositoryText("host.055") : errorText(cause) });
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
  useCommonTranslation();
  const sample = useContext(Context);
  if (!sample) return null;
  if (!sample.monitor) return <span className="text-xs text-muted-foreground" title={sample.error}>{sample.error ? hostRepositoryText("host.056") : hostRepositoryText("host.057")}</span>;
  const monitor = sample.monitor;
  const diskUsed = monitor.disks.reduce((sum, disk) => sum + disk.used, 0);
  const diskTotal = monitor.disks.reduce((sum, disk) => sum + disk.total, 0);
  return <span aria-label={hostRepositoryText("host.058")} className="inline-flex items-center gap-3 text-xs tabular-nums text-muted-foreground" title={hostRepositoryText("host.059")}>
    <span className="inline-flex items-center gap-1" title={hostRepositoryText("host.034")}><Activity className="size-3.5" />CPU {monitor.cpu_percent.toFixed(0)}%</span>
    <span className="inline-flex items-center gap-1" title={hostRepositoryText("host.060", {value1: (monitor.memory_used / 1024 ** 3).toFixed(1), value2: (monitor.memory_total / 1024 ** 3).toFixed(1)})}><MemoryStick className="size-3.5" />{hostRepositoryText("host.039")}{" "}{percent(monitor.memory_used, monitor.memory_total)}</span>
    <span className="inline-flex items-center gap-1" title={monitor.disks.map(disk => `${disk.mount || disk.name}：${percent(disk.used, disk.total)}`).join("\n")}><HardDrive className="size-3.5" />{hostRepositoryText("host.042")}{" "}{percent(diskUsed, diskTotal)}</span>
  </span>;
}
