import { commonText, useCommonTranslation } from "../i18n";
import { currentLocale } from "../locale";
import { AutoField } from "./auto-form";
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
  if (parts.length !== 5 && parts.length !== 6) return commonText("cron.fields");
  const fields = parts.length === 6 ? parts : ["0", ...parts];
  const ranges: [number, number, string[]?][] = [
    [0, 59], [0, 59], [0, 23], [1, 31],
    [1, 12, ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"]],
    [0, 7, ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"]],
  ];
  const invalid = fields.findIndex((part, index) => part === "?" ? index !== 3 && index !== 5 : !validPart(part!, ...ranges[index]!));
  return invalid < 0 ? "" : commonText("cron.invalidField", { field: invalid + (parts.length === 5 ? 0 : 1) });
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
  useCommonTranslation();
  const error = cronError(value);
  const parsed = readCycle(value);
  useEffect(() => onValidityChange(!!error), [error, onValidityChange]);
  const number = (label: string, current: number, min: number, max: number, key: "interval" | "hour" | "minute" | "day") =>
    <AutoField label={<>{label}</>} className="flex min-w-24 flex-col gap-1 text-sm"><Input aria-label={label} type="number" min={min} max={max} value={current} readOnly={readOnly} onChange={event => {
      const next = Number(event.target.value);
      if (!Number.isInteger(next) || next < min || next > max) return;
      onChange(expression(parsed.cycle, key === "interval" ? next : parsed.interval, key === "hour" ? next : parsed.hour, key === "minute" ? next : parsed.minute, key === "day" ? next : parsed.day));
    }} /></AutoField>;
  return <div className="space-y-3">
    <div className="flex flex-wrap items-end gap-3">
      <AutoField label={commonText("cron.schedule")}><select aria-label={commonText("cron.cycleType")} className="h-9 min-w-32 rounded-md border border-input bg-background px-3 text-sm" disabled={readOnly} value={parsed.cycle} onChange={event => onChange(expression(event.target.value as Cycle, 1, 0, 0, 1) || value)}>
        <option value="minutes">{commonText("cron.everyMinutes")}</option><option value="hours">{commonText("cron.everyHours")}</option><option value="daily">{commonText("cron.daily")}</option><option value="weekly">{commonText("cron.weekly")}</option><option value="monthly">{commonText("cron.monthly")}</option><option value="custom" disabled>{commonText("cron.custom")}</option>
      </select></AutoField>
      {parsed.cycle === "minutes" && number(commonText("cron.minuteInterval"), parsed.interval, 1, 59, "interval")}
      {parsed.cycle === "hours" && number(commonText("cron.hourInterval"), parsed.interval, 1, 23, "interval")}
      {(["daily", "weekly", "monthly"] as Cycle[]).includes(parsed.cycle) && <>{number(commonText("cron.hour"), parsed.hour, 0, 23, "hour")}{number(commonText("cron.minute"), parsed.minute, 0, 59, "minute")}</>}
      {parsed.cycle === "weekly" && <AutoField label={commonText("cron.weekday")} className="flex flex-col gap-1 text-sm"><select aria-label={commonText("cron.weekday")} className="h-9 rounded-md border border-input bg-background px-3" disabled={readOnly} value={parsed.day} onChange={event => onChange(expression("weekly", 1, parsed.hour, parsed.minute, Number(event.target.value)))}>{days.map((day, index) => <option key={day} value={index}>{new Intl.DateTimeFormat(currentLocale(), { weekday: "short", timeZone: "UTC" }).format(Date.UTC(2023, 0, index + 1))}</option>)}</select></AutoField>}
      {parsed.cycle === "monthly" && number(commonText("cron.day"), parsed.day, 1, 31, "day")}
      {parsed.cycle === "custom" && <span className="pb-2 text-sm text-muted-foreground">{commonText("cron.customHelp")}</span>}
    </div>
    <AutoField label={commonText("cron.expression")} className="flex flex-col gap-1 text-sm font-medium"><Input aria-label={commonText("cron.expression")} className="font-mono" value={value} readOnly={readOnly} aria-invalid={!!error} onChange={event => onChange(event.target.value)} /></AutoField>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}
