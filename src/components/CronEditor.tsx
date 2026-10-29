import { useEffect } from "react";
import { Input } from "@/components/ui/input";

type Cycle = "minutes" | "hours" | "daily" | "weekly" | "monthly" | "custom";

const days = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

function validPart(part: string, min: number, max: number, names: string[] = []): boolean {
  const value = (token: string) => {
    const named = names.indexOf(token.toUpperCase());
    if (named >= 0) return named + min;
    return /^\d+$/.test(token) ? Number(token) : NaN;
  };
  return part.split(",").every(segment => {
    const [range, step, extra] = segment.split("/");
    if (!range || extra || (step !== undefined && (!/^\d+$/.test(step) || Number(step) < 1 || Number(step) > max - min + 1))) return false;
    if (range === "*") return true;
    const bounds = range.split("-");
    if (bounds.length > 2 || bounds.some(bound => !Number.isInteger(value(bound)) || value(bound) < min || value(bound) > max)) return false;
    return bounds.length === 1 || value(bounds[0]!) <= value(bounds[1]!);
  });
}

export function cronError(expression: string): string {
  const parts = expression.trim().split(/\s+/);
  if (parts.length !== 5 && parts.length !== 6) return "Cron 需要 5 字段（分 时 日 月 周）或 6 字段（秒 分 时 日 月 周）。";
  const fields = parts.length === 6 ? parts : ["0", ...parts];
  const ranges: [number, number, string[]?][] = [
    [0, 59], [0, 59], [0, 23], [1, 31],
    [1, 12, ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"]],
    [0, 7, ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"]],
  ];
  const invalid = fields.findIndex((part, index) => part === "?" ? index !== 3 && index !== 5 : !validPart(part!, ...ranges[index]!));
  return invalid < 0 ? "" : `Cron 第 ${invalid + (parts.length === 5 ? 0 : 1)} 个字段无效。`;
}

function readCycle(expression: string): { cycle: Cycle; interval: number; hour: number; minute: number; day: number } {
  const parts = expression.trim().split(/\s+/);
  const [minute, hour, day, month, weekday] = parts.length === 5 ? parts : [];
  const common = { interval: 1, hour: Number(hour) || 0, minute: Number(minute) || 0, day: Number(day) || 1 };
  if (/^\*\/\d+$/.test(minute ?? "") && hour === "*" && day === "*" && month === "*" && weekday === "*") return { ...common, cycle: "minutes", interval: Number(minute!.slice(2)) };
  if (minute === "0" && (hour === "*" || /^\*\/\d+$/.test(hour ?? "")) && day === "*" && month === "*" && weekday === "*") return { ...common, cycle: "hours", interval: hour === "*" ? 1 : Number(hour!.slice(2)) };
  if (/^\d+$/.test(minute ?? "") && /^\d+$/.test(hour ?? "") && month === "*") {
    if (day === "*" && weekday === "*") return { ...common, cycle: "daily" };
    if (day === "*" && /^[0-6]$/.test(weekday ?? "")) return { ...common, cycle: "weekly", day: Number(weekday) };
    if (/^\d+$/.test(day ?? "") && weekday === "*") return { ...common, cycle: "monthly" };
  }
  return { ...common, cycle: "custom" };
}

function expression(cycle: Cycle, interval: number, hour: number, minute: number, day: number): string {
  switch (cycle) {
    case "minutes": return `*/${interval} * * * *`;
    case "hours": return `0 */${interval} * * *`;
    case "daily": return `${minute} ${hour} * * *`;
    case "weekly": return `${minute} ${hour} * * ${day}`;
    case "monthly": return `${minute} ${hour} ${day} * *`;
    case "custom": return "";
  }
}

export function CronEditor({ value, readOnly, onChange, onValidityChange }: {
  value: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  onValidityChange: (invalid: boolean) => void;
}) {
  const error = cronError(value);
  const parsed = readCycle(value);
  useEffect(() => onValidityChange(!!error), [error, onValidityChange]);
  const number = (label: string, current: number, min: number, max: number, key: "interval" | "hour" | "minute" | "day") =>
    <label className="flex min-w-24 flex-col gap-1 text-sm">{label}<Input aria-label={label} type="number" min={min} max={max} value={current} readOnly={readOnly} onChange={event => {
      const next = Number(event.target.value);
      if (!Number.isInteger(next) || next < min || next > max) return;
      onChange(expression(parsed.cycle, key === "interval" ? next : parsed.interval, key === "hour" ? next : parsed.hour, key === "minute" ? next : parsed.minute, key === "day" ? next : parsed.day));
    }} /></label>;
  return <div className="space-y-3">
    <label className="block text-sm font-medium" htmlFor="cron-cycle">执行周期</label>
    <div className="flex flex-wrap items-end gap-3">
      <select id="cron-cycle" aria-label="执行周期类型" className="h-9 min-w-32 rounded-md border border-input bg-background px-3 text-sm" disabled={readOnly} value={parsed.cycle} onChange={event => onChange(expression(event.target.value as Cycle, 1, 0, 0, 1) || value)}>
        <option value="minutes">每隔几分钟</option><option value="hours">每隔几小时</option><option value="daily">每天</option><option value="weekly">每周</option><option value="monthly">每月</option><option value="custom" disabled>自定义表达式</option>
      </select>
      {parsed.cycle === "minutes" && number("间隔分钟", parsed.interval, 1, 59, "interval")}
      {parsed.cycle === "hours" && number("间隔小时", parsed.interval, 1, 23, "interval")}
      {(["daily", "weekly", "monthly"] as Cycle[]).includes(parsed.cycle) && <>{number("小时", parsed.hour, 0, 23, "hour")}{number("分钟", parsed.minute, 0, 59, "minute")}</>}
      {parsed.cycle === "weekly" && <label className="flex flex-col gap-1 text-sm">星期<select aria-label="星期" className="h-9 rounded-md border border-input bg-background px-3" disabled={readOnly} value={parsed.day} onChange={event => onChange(expression("weekly", 1, parsed.hour, parsed.minute, Number(event.target.value)))}>{days.map((day, index) => <option key={day} value={index}>{day}</option>)}</select></label>}
      {parsed.cycle === "monthly" && number("日期", parsed.day, 1, 31, "day")}
      {parsed.cycle === "custom" && <span className="pb-2 text-sm text-muted-foreground">此表达式使用自定义规则</span>}
    </div>
    <label className="flex flex-col gap-1 text-sm font-medium">Cron 表达式<Input aria-label="Cron 表达式" className="font-mono" value={value} readOnly={readOnly} aria-invalid={!!error} onChange={event => onChange(event.target.value)} /></label>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}
