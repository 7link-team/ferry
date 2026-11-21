import { useEffect } from "react";
import { AutoField } from "../shared/forms/auto-form";
import { Input } from "../shared/ui/input";
import { commonText, useCommonTranslation } from "../shared/i18n/i18n";

export const resourceLimitGroups = [
  { key: "connections", keys: ["http_body_bytes", "websocket_message_bytes", "cluster_message_bytes", "preview_connections_per_entry", "preview_connections_per_host", "preview_body_bytes", "host_task_concurrency", "pipeline_concurrent_runs", "conversation_concurrent_runs", "workflow_concurrent_runs", "cron_concurrent_runs"] },
  { key: "content", keys: ["tool_request_bytes", "tool_response_bytes", "agent_native_line_bytes", "agent_event_bytes", "agent_turn_output_bytes", "agent_prompt_bytes", "agent_schema_bytes", "agent_tool_calls", "agent_tool_concurrency", "provider_concurrency", "provider_requests_per_second", "task_timeout_seconds", "file_upload_bytes", "file_count", "file_total_bytes", "directory_entries", "command_output_bytes", "command_bytes", "log_output_bytes"] },
  { key: "workflows", keys: ["workflow_definition_bytes", "workflow_pinned_bytes", "workflow_nodes", "workflow_node_executions", "workflow_code_bytes", "workflow_packet_bytes", "workflow_items", "workflow_platform_pages", "workflow_evaluations", "sandbox_memory_bytes", "sandbox_process_memory_bytes", "sandbox_timeout_ms", "report_input_bytes", "report_pages", "report_sections", "report_metrics", "report_concurrency", "report_timeout_seconds"] },
] as const;

type Props = {
  draft: Record<string, unknown>;
  readOnly: boolean;
  onPatch: (patch: { resource_limits: Record<string, unknown> }) => void;
  onValidityChange?: (blocked: boolean) => void;
};
const text = (key: string) => commonText(`resource-domain:platform.resourceLimits.${key}`);
const valid = (value: unknown) => value == null || (typeof value === "number" && Number.isSafeInteger(value) && value > 0);

export function PlatformResourceLimits({ draft, readOnly, onPatch, onValidityChange }: Props) {
  useCommonTranslation();
  const limits = (draft.resource_limits ?? {}) as Record<string, unknown>;
  const blocked = Object.values(limits).some(value => !valid(value));
  useEffect(() => { onValidityChange?.(blocked); }, [blocked, onValidityChange]);
  const patch = (key: string, value: unknown) => onPatch({ resource_limits: { ...limits, [key]: value } });
  return <div className="min-w-0 space-y-6" data-resource-limits>
    <p className="text-sm leading-relaxed text-muted-foreground">{text("help")}</p>
    {resourceLimitGroups.map(group => <fieldset key={group.key} className="min-w-0 space-y-4">
      <legend className="mb-3 text-sm font-medium">{text(group.key)}</legend>
      <div className="grid min-w-0 gap-4 xl:grid-cols-2">
        {group.keys.map(key => {
          const value = limits[key];
          const unit = key.endsWith("_bytes") ? "bytes" : key.endsWith("_seconds") ? "seconds" : key.endsWith("_ms") ? "milliseconds" : "count";
          return <AutoField key={key} name={`resource_limits.${key}`} label={text(`field.${key}`)} help={text(unit)} error={!valid(value) ? text("invalid") : undefined} data-resource-limit={key}>
            {readOnly ? <span className="min-h-11 break-words py-2.5 text-sm">{value == null ? text("unlimited") : String(value)}</span> : <div className="flex min-w-0 flex-wrap items-center gap-2">
              <select className="min-h-11 max-w-full rounded-md border border-input bg-background px-3 text-sm" value={value == null ? "unlimited" : "custom"} onChange={event => patch(key, event.target.value === "unlimited" ? null : 1)}>
                <option value="unlimited">{text("unlimited")}</option><option value="custom">{text("custom")}</option>
              </select>
              {value != null && <Input type="number" min={1} step={1} max={Number.MAX_SAFE_INTEGER} className="min-h-11 w-40 max-w-full grow basis-32" aria-label={`${text(`field.${key}`)} (${text(unit)})`} aria-invalid={!valid(value)} value={String(value)} onChange={event => { const raw = event.target.value; const n = Number(raw); patch(key, raw !== "" && Number.isSafeInteger(n) && n > 0 ? n : raw); }} />}
            </div>}
          </AutoField>;
        })}
      </div>
    </fieldset>)}
  </div>;
}
