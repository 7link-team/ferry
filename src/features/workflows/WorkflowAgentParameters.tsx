import { useContext, useEffect, useRef, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { cliToolLabel } from "../../shared/forms/cli-tools";
import { FieldCaption } from "../../shared/forms/FieldHelp";
import { workflowText as t, commonText } from "../../shared/i18n/i18n";
import { Input } from "../../shared/ui/input";
import { WorkflowAssetContext } from "./WorkflowAssets";
import type { GraphNode } from "./graph";
import { InlineAlert } from "../../shared/ui/inline-alert";

type Params = Record<string, unknown>;
export type AgentOption = {
  id: string;
  name: string;
  state: string;
  runtime: string;
  host_id?: string;
  host_name?: string;
};

const control = "w-full min-w-0 rounded-md border border-input bg-background px-3 text-sm";
/** CLIs that cannot run without write access (server: conversations.rs). */
const READ_ONLY_UNSUPPORTED = new Set(["kimi", "grok"]);

/** The server's save-time rule for the output contract; "" when acceptable. */
export function schemaProblem(source: string): "" | "agent.schemaJson" | "agent.schemaRoot" | "agent.schemaSize" {
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    return "agent.schemaJson";
  }
  if (!value || typeof value !== "object" || Array.isArray(value) || (value as Params).type !== "object") return "agent.schemaRoot";
  return "";
}

const usable = (agent: AgentOption) => agent.state === "enabled" && !READ_ONLY_UNSUPPORTED.has(agent.runtime);
/** CLIs whose writes the Runner can confine to a working directory (server: agent_files.rs). */
const FILE_RUNTIMES = new Set(["codex", "claude"]);
type Files = { enabled?: boolean; formats?: string[] | "any"; max_files?: number; max_total_mb?: number; instructions?: string };

/** "pdf, .PPTX" → ["pdf", "pptx"]; empty means any format. */
export function parseFormats(text: string): string[] {
  const seen: string[] = [];
  for (const part of text.split(/[\s,，;；]+/)) {
    const value = part.trim().replace(/^\./, "").toLowerCase();
    if (value && !seen.includes(value)) seen.push(value);
  }
  return seen;
}
export const formatsProblem = (formats: string[]) =>
  formats.some((format) => !/^[a-z0-9]{1,10}$/.test(format));

export function WorkflowAgentParameters({ node, readOnly, patch, invalid }: {
  node: GraphNode;
  readOnly: boolean;
  patch: (value: Params) => void;
  invalid: (key: string, value: boolean) => void;
}) {
  const { workspace } = useContext(WorkflowAssetContext);
  const p = node.params;
  const settings = (node.settings ?? {}) as Params;
  const target = settings.runtime !== "server" && settings.target_ref ? String(settings.target_ref) : "";
  const [agents, setAgents] = useState<AgentOption[]>();
  const [hosts, setHosts] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true;
    api<{ items: AgentOption[] }>(`${base(workspace, "ai-agents")}?page_size=100`)
      .then((result) => { if (live) setAgents(result.items); })
      .catch((cause) => { if (live) setError(errorText(cause)); });
    return () => { live = false; };
  }, [workspace]);
  useEffect(() => {
    if (!target || hosts[target]) return;
    let live = true;
    api<{ items: { id: string; name: string }[] }>(`${base(workspace, "hosts")}?page_size=200`)
      .then((result) => { if (live) setHosts(Object.fromEntries(result.items.map((host) => [host.id, host.name]))); })
      .catch(() => {});
    return () => { live = false; };
  }, [workspace, target, hosts]);

  const agentId = String(p.agent_id ?? "");
  const agent = agents?.find((row) => row.id === agentId);
  const hostName = target ? hosts[target] ?? target : agent?.host_id && agent.host_id !== "0" ? agent.host_name || agent.host_id : "";
  const location = agent?.runtime === "api" ? t("agent.locationApi") : agent && hostName
    ? t("agent.location").replace("{host}", hostName).replace("{cli}", cliToolLabel(agent.runtime))
    : t(agent ? "agent.locationAutomatic" : "agent.locationPending");

  const prompt = String(p.prompt ?? "");
  const promptTooLong = false;
  const files = (p.files ?? {}) as Files;
  const filesOn = files.enabled === true;
  const [formatsText, setFormatsText] = useState(Array.isArray(files.formats) ? files.formats.join(", ") : "");
  const formats = parseFormats(formatsText);
  const badFormats = filesOn && formatsProblem(formats);
  const patchFiles = (next: Partial<Files>) => patch({ files: { ...files, ...next } });
  const serialized = p.output_schema === undefined ? (filesOn ? "" : "{}") : JSON.stringify(p.output_schema, null, 2);
  const [source, setSource] = useState(serialized);
  const committed = useRef(serialized);
  useEffect(() => {
    if (serialized !== committed.current) {
      committed.current = serialized;
      setSource(serialized);
    }
  }, [serialized]);
  // With file output the JSON answer is optional: an empty schema is allowed.
  const problem = filesOn && source.trim() === "" ? "" : schemaProblem(source);
  const fileRuntimeProblem = filesOn && !!agent && !FILE_RUNTIMES.has(agent.runtime);
  const report = useRef(invalid);
  report.current = invalid;
  useEffect(() => { report.current("agentSchema", problem !== ""); }, [problem]);
  useEffect(() => { report.current("agentPrompt", promptTooLong); }, [promptTooLong]);
  useEffect(() => { report.current("agentFiles", badFormats || fileRuntimeProblem); }, [badFormats, fileRuntimeProblem]);
  useEffect(() => () => { report.current("agentSchema", false); report.current("agentPrompt", false); report.current("agentFiles", false); }, []);
  const number = (key: "timeout_s" | "max_tool_calls" | "max_tokens", label: Parameters<typeof t>[0], min: number, max?: number) => (
    <label className="block min-w-0 space-y-2 text-sm">
      <span className="block font-medium"><FieldCaption label={<>{t(label)}</>} /></span>
      <Input aria-label={t(label)} type="number" min={min} max={max} step="1" readOnly={readOnly}
        value={String(p[key] ?? "")} onChange={(event) => patch({ [key]: event.target.value === "" ? undefined : Number(event.target.value) })} />
    </label>
  );
  return <>
    <p className="text-sm leading-6 text-muted-foreground" data-testid="agent-location">{location}</p>
    <label className="block min-w-0 space-y-2 text-sm">
      <span className="block font-medium"><FieldCaption label={<>{t("agent.agent")}</>} /></span>
      <select aria-label={t("agent.agent")} className={`${control} h-10`} disabled={readOnly} required value={agentId}
        onChange={(event) => patch({ agent_id: event.target.value })}>
        <option value="">{t("agent.select")}</option>
        {agentId && agents && !agent && <option value={agentId} disabled>{agentId}</option>}
        {agents?.map((row) => (
          <option key={row.id} value={row.id} disabled={!usable(row)}>
            {row.name} · {cliToolLabel(row.runtime)}{row.state !== "enabled" ? ` · ${t("agent.disabled")}` : READ_ONLY_UNSUPPORTED.has(row.runtime) ? ` · ${t("agent.readOnlyUnsupported")}` : ""}
          </option>
        ))}
      </select>
    </label>
    {agent && !usable(agent) && <p role="alert" className="text-sm text-destructive">{READ_ONLY_UNSUPPORTED.has(agent.runtime) ? t("agent.readOnlyUnsupported") : t("agent.disabled")}</p>}
    {error && <InlineAlert>{error}</InlineAlert>}
    <label className="block min-w-0 space-y-2 text-sm">
      <span className="block font-medium"><FieldCaption label={<>{t("agent.prompt")}</>} /></span>
      <textarea aria-label={t("agent.prompt")} className={`${control} min-h-32 py-3`} readOnly={readOnly} required value={prompt}
        onChange={(event) => patch({ prompt: event.target.value })} />
    </label>
    {promptTooLong && <p role="alert" className="text-sm text-destructive">{t("agent.promptTooLong")}</p>}
    <p className="text-xs leading-5 text-muted-foreground">{t("agent.promptHelp")}</p>
    <label className="flex min-h-11 items-center gap-3 text-sm font-medium">
      <input type="checkbox" aria-label={t("agent.files")} className="size-4" disabled={readOnly} readOnly={readOnly} checked={filesOn}
        onChange={(event) => patchFiles({ enabled: event.target.checked })} />
      {t("agent.files")}
    </label>
    <p className="text-xs leading-5 text-muted-foreground">{t("agent.filesHelp")}</p>
    {filesOn && <>
      {fileRuntimeProblem && <p role="alert" className="text-sm text-destructive">{t("agent.filesRuntime")}</p>}
      {agent?.runtime === "claude" && <p className="text-xs leading-5 text-amber-700 dark:text-amber-400">{t("agent.filesClaude")}</p>}
      <label className="block min-w-0 space-y-2 text-sm">
        <span className="block font-medium"><FieldCaption label={<>{t("agent.filesFormats")}</>} /></span>
        <Input aria-label={t("agent.filesFormats")} readOnly={readOnly} value={formatsText} placeholder="pdf, pptx, xlsx"
          onChange={(event) => {
            setFormatsText(event.target.value);
            const next = parseFormats(event.target.value);
            if (!formatsProblem(next)) patchFiles({ formats: next.length ? next : "any" });
          }} />
      </label>
      {badFormats && <p role="alert" className="text-sm text-destructive">{t("agent.filesFormatsInvalid")}</p>}
      <p className="text-xs leading-5 text-muted-foreground">{t("agent.filesFormatsHelp")}</p>
      <label className="block min-w-0 space-y-2 text-sm">
        <span className="block font-medium"><FieldCaption label={<>{t("agent.filesInstructions")}</>} /></span>
        <textarea aria-label={t("agent.filesInstructions")} className={`${control} min-h-20 py-3`} readOnly={readOnly}
          value={String(files.instructions ?? "")} onChange={(event) => patchFiles({ instructions: event.target.value })} />
      </label>
    </>}
    <label className="block min-w-0 space-y-2 text-sm">
      <span className="block font-medium"><FieldCaption label={<>{t("agent.outputSchema")}</>} /></span>
      <textarea aria-label={t("agent.outputSchema")} className={`${control} min-h-48 py-3 font-mono text-xs`} readOnly={readOnly} spellCheck={false} value={source}
        onChange={(event) => {
          setSource(event.target.value);
          if (filesOn && event.target.value.trim() === "") {
            committed.current = "";
            patch({ output_schema: undefined });
          } else if (schemaProblem(event.target.value) === "") {
            const next = JSON.parse(event.target.value) as Params;
            committed.current = JSON.stringify(next, null, 2);
            patch({ output_schema: next });
          }
        }} />
    </label>
    {problem && <p role="alert" className="text-sm text-destructive">{t(problem)}</p>}
    <p className="text-xs leading-5 text-muted-foreground">{t(filesOn ? "agent.schemaOptional" : "agent.schemaHelp")}</p>
    {agent?.runtime === "api" && number("max_tokens", "aiMaxTokens", 1)}
    <p className="text-xs leading-5 text-muted-foreground">{commonText("resource-domain:platform.resourceLimits.help")}</p>
    <p className="text-xs leading-5 text-muted-foreground">{t("agent.help")}</p>
  </>;
}
