import { useEffect, useRef, useState } from "react";
import { commonText } from "../i18n/i18n";
import { localDateTime, parseLocalDateTime, parseAbsoluteDateTime, useWorkspaceTimezone } from "../timezone";
import { Input } from "../ui/input";
import { AutoField } from "./auto-form";

/** A business wall clock with a strict absolute-ISO escape for DST overlap/gaps. */
export function WorkspaceDateTimeInput({ value, readOnly, onChange, onValidityChange }: { value: string; readOnly: boolean; onChange: (value: string) => void; onValidityChange: (blocked: boolean) => void }) {
  const timezone = useWorkspaceTimezone();
  const validity = useRef(onValidityChange);
  validity.current = onValidityChange;
  const [wall, setWall] = useState(() => localDateTime(value, timezone));
  const [absolute, setAbsolute] = useState(value);
  const [error, setError] = useState(false);
  useEffect(() => { setWall(localDateTime(value, timezone)); setAbsolute(value); setError(false); }, [value, timezone]);
  useEffect(() => { validity.current(error && !readOnly); }, [error, readOnly]);
  useEffect(() => () => validity.current(false), []);
  return <div className="space-y-2">
    <Input aria-label={commonText("resource-domain:workspace.wallTime")} type="datetime-local" readOnly={readOnly} value={wall} onChange={event => {
      const next = event.target.value;
      setWall(next);
      const parsed = next ? parseLocalDateTime(next, timezone) : "";
      setError(parsed === null);
      if (parsed !== null) onChange(parsed);
    }} />
    <p className="break-words text-xs text-muted-foreground">{commonText(timezone ? "resource-domain:workspace.effectiveTimezone" : "resource-domain:workspace.deviceTime", { zone: timezone })}</p>
    {error && <p role="alert" className="text-sm text-destructive">{commonText("resource-domain:workspace.invalidWallTime")}</p>}
    {timezone && <AutoField label={commonText("resource-domain:workspace.absoluteTime")} help={commonText("resource-domain:workspace.absoluteTimeHelp")}><Input readOnly={readOnly} value={absolute} onChange={event => {
      const next = event.target.value;
      setAbsolute(next);
      const parsed = next ? parseAbsoluteDateTime(next) : "";
      setError(parsed === null);
      if (parsed !== null) onChange(parsed);
    }} /></AutoField>}
  </div>;
}
