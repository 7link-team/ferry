import { useEffect } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText, executionText } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";

export type ExecutionPolicy = { enabled: boolean; shared: boolean; capacity: number };
export type ExecutionStatus = {
  active: number; capacity: number;
  state: "disabled" | "draining" | "busy" | "available";
  running_jobs: { run_id: string; job_key: string }[];
};
export const defaultExecutionPolicy: ExecutionPolicy = { enabled: true, shared: false, capacity: 1 };

export function ExecutionPolicyFields({ value = defaultExecutionPolicy, readOnly, onChange, onValidityChange }: {
  value?: ExecutionPolicy | undefined; readOnly: boolean;
  onChange: (value: ExecutionPolicy) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  const invalid = !Number.isInteger(value.capacity) || value.capacity < 1 || value.capacity > 64;
  useEffect(() => {
    onValidityChange(!readOnly && invalid);
    return () => onValidityChange(false);
  }, [invalid, readOnly, onValidityChange]);
  if (readOnly) return <dl className="flex flex-wrap gap-x-8 gap-y-4 text-sm">
    {[
      [executionText("hostExecution.enabled"), commonText(value.enabled ? "common.yes" : "common.no")],
      [executionText("hostExecution.shared"), commonText(value.shared ? "common.yes" : "common.no")],
      [executionText("hostExecution.capacity"), value.capacity],
    ].map(([label, content]) => <div key={label} className="space-y-1">
      <dt className="text-muted-foreground">{label}</dt><dd>{content}</dd>
    </div>)}
  </dl>;
  return <div className="flex flex-wrap items-end gap-x-8 gap-y-4">
    <label className="flex min-h-9 items-center gap-2 text-sm"><input type="checkbox" disabled={readOnly} checked={value.enabled} onChange={event => onChange({ ...value, enabled: event.target.checked })} />{executionText("hostExecution.enabled")}</label>
    <label className="flex min-h-9 items-center gap-2 text-sm"><input type="checkbox" disabled={readOnly} checked={value.shared} onChange={event => onChange({ ...value, shared: event.target.checked })} />{executionText("hostExecution.shared")}</label>
    <AutoField label={executionText("hostExecution.capacity")}><Input className="w-24" type="number" min={1} max={64} step={1} aria-label={executionText("hostExecution.capacity")} aria-invalid={invalid} disabled={readOnly} value={Number.isNaN(value.capacity) ? "" : value.capacity} onChange={event => onChange({ ...value, capacity: event.target.valueAsNumber })} /></AutoField>
  </div>;
}
