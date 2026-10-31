import { useState } from "react";

export type MonitorPoint = { at: number; value: number | null };
export type MonitorSeries = { name: string; points: MonitorPoint[]; secondary?: boolean };

const clock = (at: number, seconds: boolean) => new Date(at).toLocaleTimeString([], {
  hour: "2-digit", minute: "2-digit", ...(seconds ? { second: "2-digit" } : {}), hour12: false,
});

/** Time coordinates always cover the requested window; absent samples stay absent. */
export function MonitorTimeChart({ label, series, start, end, step, ceiling, format }: {
  label: string; series: MonitorSeries[]; start: number; end: number; step: number;
  ceiling?: number; format: (value: number) => string;
}) {
  const [selected, setSelected] = useState<number>();
  const points = series[0]?.points ?? [];
  const max = ceiling ?? Math.max(1, ...series.flatMap(item => item.points.map(point => point.value ?? 0))) * 1.1;
  const x = (at: number) => (at - start) / Math.max(1, end - start) * 600;
  const y = (value: number) => 120 - Math.max(0, Math.min(max, value)) / max * 120;
  const visible = (point: MonitorPoint) => point.at >= start && point.at <= end;
  const segments = (values: MonitorPoint[]) => {
    const paths: MonitorPoint[][] = [];
    let current: MonitorPoint[] = [];
    for (const point of values) {
      if (!visible(point) || point.value === null || !Number.isFinite(point.value)) {
        current = []; continue;
      }
      if (!current.length || point.at - current.at(-1)!.at > step * 1.5) {
        current = []; paths.push(current);
      }
      current.push(point);
    }
    return paths;
  };
  const selectedPoint = selected === undefined ? undefined : points[selected];
  const active = selectedPoint && visible(selectedPoint) ? selectedPoint : undefined;
  const ticks: number[] = [];
  for (let at = Math.ceil(start / step) * step; at <= end; at += step) ticks.push(at);
  const hasValues = series.some(item => item.points.some(point => visible(point) && point.value !== null));
  const showSeconds = end - start <= 15 * 60_000;
  return <div className="relative space-y-2" data-monitor-chart={label}>
    <div className="relative pl-12 pr-2">
      <div aria-hidden="true" className="pointer-events-none absolute inset-y-0 left-0 flex w-11 flex-col justify-between pr-1 text-right text-xs tabular-nums text-muted-foreground">
        <span>{format(max)}</span><span>{format(max / 2)}</span><span>{format(0)}</span>
      </div>
      <svg role="img" aria-label={`${label}，${clock(start, true)} 至 ${clock(end, true)}，${points.filter(point => visible(point) && point.value !== null).length} 个真实采样点。左右方向键查看采样。`}
        tabIndex={0} viewBox="0 0 600 120" preserveAspectRatio="none"
        className="h-32 w-full rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
        onFocus={() => { if (points.length) setSelected(points.length - 1); }}
        onBlur={() => setSelected(undefined)} onPointerLeave={() => setSelected(undefined)}
        onPointerMove={event => {
          if (!points.length) return;
          const rect = event.currentTarget.getBoundingClientRect();
          if (!rect.width) return;
          const at = start + (event.clientX - rect.left) / rect.width * (end - start);
          let closest = 0;
          points.forEach((point, index) => { if (Math.abs(point.at - at) < Math.abs(points[closest]!.at - at)) closest = index; });
          setSelected(closest);
        }} onKeyDown={event => {
          if (!points.length) return;
          if (event.key === "Escape") { setSelected(undefined); return; }
          if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
          event.preventDefault();
          setSelected(current => event.key === "Home" ? 0 : event.key === "End" ? points.length - 1 : Math.max(0, Math.min(points.length - 1, (current ?? points.length - 1) + (event.key === "ArrowLeft" ? -1 : 1))));
        }}>
        {[0, 60, 120].map(value => <line key={value} x1="0" x2="600" y1={value} y2={value} stroke="var(--border)" strokeDasharray={value === 120 ? undefined : "3 5"} vectorEffect="non-scaling-stroke" />)}
        <path d={ticks.map(at => `M${x(at)},117v3`).join(" ")} stroke="var(--muted-foreground)" strokeOpacity="0.35" vectorEffect="non-scaling-stroke" />
        {series.map(item => <g key={item.name} stroke={item.secondary ? "var(--success)" : "var(--primary)"}>
          {segments(item.points).map((segment, index) => segment.length === 1
            ? <circle key={index} data-sample-point cx={x(segment[0]!.at)} cy={y(segment[0]!.value!)} r="3" fill={item.secondary ? "var(--success)" : "var(--primary)"} />
            : <polyline key={index} data-sample-line points={segment.map(point => `${x(point.at)},${y(point.value!)}`).join(" ")} fill="none" strokeWidth="2" strokeDasharray={item.secondary ? "5 3" : undefined} vectorEffect="non-scaling-stroke" strokeLinejoin="round" />)}
        </g>)}
        {active && <line x1={x(active.at)} x2={x(active.at)} y1="0" y2="120" stroke="var(--muted-foreground)" strokeDasharray="2 3" vectorEffect="non-scaling-stroke" />}
      </svg>
      {!hasValues && <span className="pointer-events-none absolute inset-0 flex items-center justify-center pl-12 text-xs text-muted-foreground">等待有效采样</span>}
      {active && <div role="tooltip" className="pointer-events-none absolute right-2 top-1 z-10 rounded-md border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-sm">
        <div className="mb-1 font-medium tabular-nums">{new Date(active.at).toLocaleString([], { hour12: false })}</div>
        {series.map(item => <div key={item.name} className="flex justify-between gap-4"><span>{item.name}</span><span className="tabular-nums">{item.points[selected!]?.value == null ? "—" : format(item.points[selected!]!.value!)}</span></div>)}
      </div>}
    </div>
    <div aria-hidden="true" className="ml-12 flex justify-between pr-2 text-xs tabular-nums text-muted-foreground">
      {[0, 1, 2, 3, 4].map(index => <span key={index} className={index % 2 ? "hidden sm:inline" : undefined}>{clock(start + (end - start) * index / 4, showSeconds)}</span>)}
    </div>
    {series.length > 1 && <div className="ml-12 flex gap-4 text-xs text-muted-foreground">{series.map(item => <span key={item.name} className="inline-flex items-center gap-1.5"><span className={`inline-block w-4 border-t-2 ${item.secondary ? "border-dashed border-success" : "border-primary"}`} />{item.name}</span>)}</div>}
  </div>;
}
