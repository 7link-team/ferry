import { Switch } from "../../shared/ui/switch";
import { useEffect } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText, executionText } from "../../shared/i18n/i18n";

export type ExecutionPolicy = { enabled: boolean; shared: boolean; capacity: number };
export type ExecutionStatus = {
  active: number; capacity: number | null;
  state: "disabled" | "draining" | "busy" | "available" | "recovering";
  running_jobs: { run_id: string; job_key: string }[];
};
export const defaultExecutionPolicy: ExecutionPolicy = { enabled: true, shared: false, capacity: 1 };

export function ExecutionPolicyFields({ value = defaultExecutionPolicy, readOnly, onChange, onValidityChange }: {
  value?: ExecutionPolicy | undefined; readOnly: boolean;
  onChange: (value: ExecutionPolicy) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  useEffect(() => {
    onValidityChange(false);
    return () => onValidityChange(false);
  }, [onValidityChange]);
  if (readOnly) return <dl className="flex flex-wrap gap-x-8 gap-y-4 text-sm">
    {[
      [executionText("hostExecution.enabled"), commonText(value.enabled ? "common.yes" : "common.no")],
      [executionText("hostExecution.shared"), commonText(value.shared ? "common.yes" : "common.no")],
    ].map(([label, content]) => <div key={label} className="space-y-1">
      <dt className="text-muted-foreground">{label}</dt><dd>{content}</dd>
    </div>)}
  </dl>;
  return <div className="flex flex-wrap items-end gap-x-8 gap-y-4">
    <AutoField label={<>{executionText("hostExecution.enabled")}</>} className="min-w-40"><Switch  disabled={readOnly} checked={value.enabled} onCheckedChange={checked => onChange({ ...value, enabled: checked })} /></AutoField>
    <AutoField label={<>{executionText("hostExecution.shared")}</>} className="min-w-40"><Switch  disabled={readOnly} checked={value.shared} onCheckedChange={checked => onChange({ ...value, shared: checked })} /></AutoField>
    <p className="basis-full text-sm text-muted-foreground">{commonText("resource-domain:platform.resourceLimits.hostHelp")}</p>
  </div>;
}
