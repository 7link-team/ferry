import { useEffect, useState } from "react";
import { LoaderCircle, Signal, SignalLow, SignalMedium, WifiOff } from "lucide-react";
import { api, errorText } from "../api";

type Measurement = { milliseconds?: number; checking: boolean; error?: string };

/** One probe for the host workspace, shared by all terminal/file tabs and splits. */
export function HostLatency({ workspace, hostId }: { workspace: string; hostId: string }) {
  const [measurement, setMeasurement] = useState<Measurement>({ checking: true });

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let request: AbortController | undefined;
    const probe = async () => {
      if (disposed || document.hidden || request) return;
      const controller = new AbortController();
      request = controller;
      const timeout = setTimeout(() => controller.abort(), 10_000);
      setMeasurement(current => ({ ...current, checking: true }));
      const started = performance.now();
      try {
        const result = await api<{ ok: boolean; stderr?: string }>(
          `/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/connection-tests`,
          { method: "POST", signal: controller.signal },
        );
        if (!result.ok) throw new Error(result.stderr || "主机连接检测失败");
        if (!disposed) setMeasurement({ milliseconds: Math.max(1, Math.round(performance.now() - started)), checking: false });
      } catch (cause) {
        if (!disposed) setMeasurement({ checking: false, error: controller.signal.aborted ? "连接检测超时（10 秒）" : errorText(cause) });
      } finally {
        clearTimeout(timeout);
        request = undefined;
        if (!disposed) timer = setTimeout(() => { void probe(); }, 10_000);
      }
    };
    const resume = () => {
      if (!document.hidden) {
        clearTimeout(timer);
        void probe();
      }
    };
    setMeasurement({ checking: true });
    void probe();
    document.addEventListener("visibilitychange", resume);
    return () => {
      disposed = true;
      clearTimeout(timer);
      request?.abort();
      document.removeEventListener("visibilitychange", resume);
    };
  }, [workspace, hostId]);

  const { milliseconds, checking, error } = measurement;
  const color = error ? "text-destructive" : milliseconds === undefined ? "text-muted-foreground"
    : milliseconds < 100 ? "text-success" : milliseconds < 300 ? "text-warning" : "text-destructive";
  const Icon = checking ? LoaderCircle : error ? WifiOff : milliseconds === undefined ? SignalLow
    : milliseconds < 100 ? Signal : milliseconds < 300 ? SignalMedium : SignalLow;
  const label = milliseconds !== undefined ? `${milliseconds} ms` : checking ? "检测中" : "连接异常";
  const title = `${error ? `${error}。` : ""}浏览器 → Ferry → 主机的往返耗时，包含连接建立和探测命令处理时间；每 10 秒检测一次。绿色 <100 ms，黄色 100–299 ms，红色 ≥300 ms。${checking ? "正在检测…" : ""}`;
  return <span title={title} aria-label={`主机连接延迟：${label}${checking ? "，正在检测" : ""}`}
    className={`inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border border-current/15 bg-current/5 px-2 text-xs tabular-nums ${color}`}>
    <Icon aria-hidden="true" className={`size-3.5 ${checking ? "motion-safe:animate-spin" : "motion-safe:animate-pulse"}`} />
    <span>{label}</span>
  </span>;
}
