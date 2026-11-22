import { createContext, useContext, useEffect, useRef, type ReactNode } from "react";
import { commonText } from "./i18n/i18n";
import { currentLocale } from "./i18n/locale";

/** null deliberately preserves each caller's historical timezone behavior. */
export const WorkspaceTimezoneContext = createContext<string | null>(null);
export const useWorkspaceTimezone = () => useContext(WorkspaceTimezoneContext);
export function detectedTimezone(): string {
  try { return Intl.DateTimeFormat().resolvedOptions().timeZone || ""; } catch { return ""; }
}
export function formatDate(value: Date | number | string, timezone: string | null, kind: "datetime" | "date" | "time" = "datetime", locale: string = currentLocale(), options: Intl.DateTimeFormatOptions = {}): string {
  const date = value instanceof Date ? value : new Date(value);
  const settings = { ...options, ...(timezone ? { timeZone: timezone } : {}) };
  return kind === "date" ? date.toLocaleDateString(locale, settings) : kind === "time" ? date.toLocaleTimeString(locale, settings) : date.toLocaleString(locale, settings);
}
export function WorkspaceDate({ value, kind, locale, options }: {value: Date | number | string; kind?: "datetime" | "date" | "time"; locale?: string; options?: Intl.DateTimeFormatOptions}): ReactNode {
  return formatDate(value, useWorkspaceTimezone(), kind, locale, options);
}
/** Calendar labels, not instants: UTC is only the neutral date arithmetic carrier. */
export function dayKey(value: Date | number, timezone: string | null, legacy: "UTC" | "device" = "UTC"): string {
  const date = value instanceof Date ? value : new Date(value);
  if (!timezone && legacy === "UTC") return date.toISOString().slice(0, 10);
  const parts = new Intl.DateTimeFormat("en-US", { ...(timezone ? { timeZone: timezone } : {}), year: "numeric", month: "2-digit", day: "2-digit" }).formatToParts(date);
  const get = (key: string) => parts.find(part => part.type === key)!.value;
  return `${get("year")}-${get("month")}-${get("day")}`;
}
function wallFormatter(timezone: string) {
  return new Intl.DateTimeFormat("en-US", { timeZone: timezone, year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
}
export function localDateTime(value: string, timezone: string | null, formatter?: Intl.DateTimeFormat): string {
  if (!value || !Number.isFinite(Date.parse(value))) return "";
  const date = new Date(value);
  if (!timezone) return new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
  const parts = (formatter ?? wallFormatter(timezone)).formatToParts(date);
  const get = (key: string) => parts.find(part => part.type === key)!.value;
  return `${get("year")}-${get("month")}-${get("day")}T${get("hour")}:${get("minute")}`;
}
/** Accept a wall clock only when exactly one UTC instant maps to it. */
export function parseLocalDateTime(value: string, timezone: string | null): string | null {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value)) return null;
  if (!timezone) { const at = new Date(value); return Number.isFinite(at.getTime()) ? at.toISOString() : null; }
  const nominal = Date.parse(`${value}:00Z`);
  if (!Number.isFinite(nominal)) return null;
  // All modern IANA offsets are minute-granular and within +/-14 hours.
  const matches: number[] = [];
  const formatter = wallFormatter(timezone);
  for (let offset = -14 * 60; offset <= 14 * 60; offset++) {
    const at = nominal + offset * 60000;
    if (localDateTime(new Date(at).toISOString(), timezone, formatter) === value) matches.push(at);
  }
  return matches.length === 1 ? new Date(matches[0]!).toISOString() : null;
}

/** Existing per-resource timezone controls remain intact only in historical mode. */
export function EffectiveTimezone({ children, onValidityChange }: { children?: ReactNode; onValidityChange?: ((blocked: boolean) => void) | undefined }) {
  const timezone = useWorkspaceTimezone();
  const validity = useRef(onValidityChange);
  validity.current = onValidityChange;
  useEffect(() => { if (timezone) validity.current?.(false); }, [timezone]);
  return timezone ? <p className="break-words text-sm text-muted-foreground">{commonText("resource-domain:workspace.effectiveTimezone", { zone: timezone })}</p> : <>{children}</>;
}

/** Next calendar-day boundary, including 23/25-hour business days. */
export function nextDayAt(now: number, timezone: string | null): number {
  const today = dayKey(now, timezone);
  let low = now, high = now + 48 * 3600000;
  while (high - low > 1) {
    const mid = Math.floor((low + high) / 2);
    if (dayKey(mid, timezone) === today) low = mid; else high = mid;
  }
  return high;
}

export function parseAbsoluteDateTime(value: string): string | null {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2}(?:\.\d{1,3})?)?(Z|[+-]\d{2}:\d{2})$/.test(value)) return null;
  const at = Date.parse(value);
  const nominal = Date.parse(value.slice(0, 16) + ":00Z");
  if (!Number.isFinite(at) || !Number.isFinite(nominal) || new Date(nominal).toISOString().slice(0, 16) !== value.slice(0, 16)) return null;
  return new Date(at).toISOString();
}
