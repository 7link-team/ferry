import { LoaderCircle,Signal,SignalLow,SignalMedium,WifiOff } from "lucide-react";
import { useEffect,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { hostRepositoryText,useCommonTranslation } from "../../shared/i18n/i18n";

type Measurement = { milliseconds?: number; checking: boolean; error?: string };

/** One probe for the host workspace, shared by all terminal/file tabs and splits. */
export function HostLatency({ workspace, hostId }: { workspace: string; hostId: string }) {
  useCommonTranslation();
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
        if (!result.ok) throw new Error(result.stderr || hostRepositoryText("host.001"));
        if (!disposed) setMeasurement({ milliseconds: Math.max(1, Math.round(performance.now() - started)), checking: false });
      } catch (cause) {
        if (!disposed) setMeasurement({ checking: false, error: controller.signal.aborted ? hostRepositoryText("host.002") : errorText(cause) });
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
  const label = milliseconds !== undefined ? `${milliseconds} ms` : checking ? hostRepositoryText("host.003") : hostRepositoryText("host.004");
  const title = [error, hostRepositoryText("host.006"), checking ? hostRepositoryText("host.005") : ""].filter(Boolean).join(" ");
  return <span title={title} aria-label={hostRepositoryText("host.007", {value1: label, value2: checking ? hostRepositoryText("host.008") : ""})}
    className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border bg-muted/60 px-2 text-xs font-semibold tabular-nums text-foreground">
    <Icon aria-hidden="true" className={`size-3.5 ${color} ${checking ? "motion-safe:animate-spin" : ""}`} />
    <span>{label}</span>
  </span>;
}
