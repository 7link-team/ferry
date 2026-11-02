import { useTranslation } from "react-i18next";
import { insightText } from "../i18n";
import { useEffect, useRef, useState } from "react";
import { AutoField } from "./auto-form";
import { Button } from "./ui/button";
import { ConnectionRepositoryFields, useConnectionRepositories } from "./PipelineRepository";
import { listResource } from "../resource";
import { errorText } from "../api";

export interface InsightRepository {
  connection_id: string;
  repository: string;
  pipeline_id: string;
  all_repositories?: boolean;
  connection_name?: string;
  pipeline_name?: string;
  provider?: string;
}
type Props = { workspace: string; value: unknown; readOnly: boolean; permissions: Set<string>; onChange: (value: unknown) => void; onValidityChange: (blocked: boolean) => void };
const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";

/** One shared form field manages memberships; connection browsing is reused from checkout. */
export function CodeInsightRepositories({ workspace, value, readOnly, permissions, onChange, onValidityChange, enabled = true }: Props & { enabled?: boolean }) {
  useTranslation("insight-domain");
  const selected = Array.isArray(value) ? value as InsightRepository[] : [];
  const canSelect = permissions.has("repositories.read");
  const canPipeline = permissions.has("pipelines.read") && permissions.has("pipelines.run");
  const [choice, setChoice] = useState({ connection_id: "", url: "" });
  const data = useConnectionRepositories(workspace, choice.connection_id, !readOnly && canSelect);
  const [pipelines, setPipelines] = useState<{ id: string; name: string }[]>([]);
  const [pipelineError, setPipelineError] = useState("");
  const [pipelineLoading, setPipelineLoading] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (readOnly || !canPipeline) return;
    const controller = new AbortController();
    setPipelineLoading(true); setPipelineError("");
    void (async () => {
      const all: { id: string; name: string }[] = [];
      for (let page = 1; ; page++) {
        const result = await listResource<{ id: string; name: string }>(workspace, "pipelines", { page, page_size: 100 }, controller.signal);
        all.push(...result.items);
        if (!result.items.length || page * 100 >= result.total) break;
      }
      if (!controller.signal.aborted) setPipelines(all);
    })().catch(cause => { if (!controller.signal.aborted) setPipelineError(errorText(cause)); })
      .finally(() => { if (!controller.signal.aborted) setPipelineLoading(false); });
    return () => controller.abort();
  }, [workspace, readOnly, canPipeline, revision]);
  const isGit = (row: InsightRepository) => row.provider === "git" || data.connections.find(connection => connection.id === row.connection_id)?.provider === "git" || row.repository.includes("://") || /^[^/@\s]+@[^/:\s]+:/.test(row.repository);
  const incomplete = !selected.length || selected.some(row => !row.connection_id || (!row.all_repositories && !row.repository.trim()) || (enabled && isGit(row) && !row.pipeline_id));
  useEffect(() => {
    onValidityChange(!readOnly && incomplete);
    return () => onValidityChange(false);
  }, [readOnly, incomplete, onValidityChange]);
  const add = () => {
    const remote = data.repositories.find(row => row.url === choice.url);
    const repository = data.connection?.provider === "git" ? choice.url.trim() : remote?.full_name ?? "";
    if (!data.connection || !repository || selected.some(row => row.connection_id === choice.connection_id && row.repository === repository)) return;
    onChange([...selected, { connection_id: choice.connection_id, repository, pipeline_id: "" }]);
    setChoice(current => ({ ...current, url: "" }));
  };
  return <section aria-label={insightText("洞察仓库")} className="space-y-4 sm:col-span-2">
    <h3 className="text-sm font-medium">{insightText("选择仓库")}</h3>
    <p className="text-xs text-muted-foreground">{insightText("可选择多个连接中的仓库；同一仓库可加入不同洞察，各洞察的作者映射独立。")}</p>
    {selected.length ? <ul className="space-y-3">{selected.map((row, index) => <li key={`${row.connection_id}/${row.repository}`} className="space-y-2 border-b pb-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="min-w-0 break-all text-sm"><span className="text-muted-foreground">{row.connection_name || data.connections.find(connection => connection.id === row.connection_id)?.name || insightText("仓库连接")} / </span>{row.all_repositories ? insightText("全部可见仓库") : row.repository}</p>
        {!readOnly && canSelect && <Button type="button" variant="ghost" size="sm" aria-label={insightText("removeRepository", { repository: row.repository })} onClick={() => onChange(selected.filter((_, position) => position !== index))}>{insightText("移除")}</Button>}
      </div>
      {!readOnly && canPipeline ? <AutoField label={insightText("分析流水线")} required={enabled && isGit(row)} className="block space-y-2"><select aria-label={insightText("pipelineFor", { repository: row.repository })} className={selectClass} value={row.pipeline_id} required={enabled && isGit(row)} disabled={pipelineLoading || !!pipelineError} onChange={event => onChange(selected.map((item, position) => position === index ? { ...item, pipeline_id: event.target.value } : item))}>
        <option value="">{isGit(row) ? insightText("请选择分析流水线") : insightText("使用平台接口")}</option>
        {row.pipeline_id && !pipelines.some(pipeline => pipeline.id === row.pipeline_id) && <option value={row.pipeline_id}>{row.pipeline_name || insightText("已配置分析流水线")}</option>}
        {pipelines.map(pipeline => <option key={pipeline.id} value={pipeline.id}>{pipeline.name}</option>)}
      </select></AutoField> : <p className="text-xs text-muted-foreground">{insightText("分析流水线：")}{row.pipeline_name || (row.pipeline_id ? insightText("已配置分析流水线") : isGit(row) ? insightText("未配置分析流水线") : insightText("使用平台接口"))}</p>}
      {isGit(row) && <p className="text-xs text-muted-foreground">{insightText("普通 Git 通过分析流水线采集；启用持续采集前须选择流水线。")}</p>}
      {!readOnly && enabled && isGit(row) && !row.pipeline_id && <p role="alert" className="text-sm text-destructive">{insightText("请为此 Git 仓库选择分析流水线，或关闭持续采集后保存。")}</p>}
    </li>)}</ul> : <p role="status" className="text-sm text-muted-foreground">{insightText("尚未选择仓库。")}</p>}
    {!readOnly && canSelect && <div className="space-y-3">
      <ConnectionRepositoryFields workspace={workspace} data={data} connectionId={choice.connection_id} repositoryUrl={choice.url} repositoryLabel={insightText("添加洞察仓库")} onChange={selection => setChoice({ connection_id: selection.connection_id, url: selection.url })} />
      <Button type="button" variant="outline" disabled={!choice.url.trim() || !data.connection || data.loading || !!data.error} onClick={add}>{insightText("加入洞察")}</Button>
    </div>}
    {!readOnly && !canSelect && <p className="text-sm text-muted-foreground">{insightText("更改仓库范围需要仓库读取权限。")}</p>}
    {!readOnly && !canPipeline && <p className="text-xs text-muted-foreground">{insightText("更换分析流水线需要流水线读取及执行权限。")}</p>}
    {!readOnly && pipelineLoading && <p role="status" className="text-sm">{insightText("正在读取分析流水线…")}</p>}
    {!readOnly && pipelineError && <div><p role="alert" className="text-sm text-destructive">{pipelineError}</p><Button type="button" variant="ghost" onClick={() => setRevision(current => current + 1)}>{insightText("重试读取流水线")}</Button></div>}
  </section>;
}

/** Keep the wire value as an array; malformed JSON never reaches the shared save. */
export function CodeInsightAliases({ value, readOnly, onChange, onValidityChange }: Omit<Props, "workspace" | "permissions">) {
  useTranslation("insight-domain");
  const serialized = JSON.stringify(Array.isArray(value) ? value : [], null, 2);
  const [text, setText] = useState(serialized);
  const committed = useRef(serialized);
  const [error, setError] = useState("");
  useEffect(() => { if (serialized !== committed.current) { committed.current = serialized; setText(serialized); setError(""); } }, [serialized]);
  useEffect(() => {
    onValidityChange(!readOnly && !!error);
    return () => onValidityChange(false);
  }, [readOnly, error, onValidityChange]);
  if (readOnly) return <AutoField label={insightText("作者昵称映射")} className="block space-y-2 sm:col-span-2"><pre className="whitespace-pre-wrap break-all text-sm">{serialized}</pre></AutoField>;
  return <AutoField label={insightText("作者昵称映射")} className="block space-y-2 sm:col-span-2">
    <textarea aria-label={insightText("作者昵称映射")} aria-invalid={!!error} className="min-h-36 w-full rounded-md border border-input bg-background p-3 font-mono text-sm" value={text} onChange={event => {
      setText(event.target.value);
      try {
        const aliases: unknown = JSON.parse(event.target.value || "[]");
        if (!Array.isArray(aliases) || aliases.some(alias => !alias || typeof alias !== "object" || typeof alias.nickname !== "string" || !alias.nickname.trim() || (!String(alias.name ?? "").trim() && !String(alias.email ?? "").trim()))) throw new Error("每条映射须填写姓名或邮箱，以及显示昵称。");
        committed.current = JSON.stringify(aliases, null, 2); setError(""); onChange(aliases);
      } catch (cause) { setError(cause instanceof SyntaxError ? "作者昵称映射须为有效的 JSON 数组。" : errorText(cause)); }
    }} />
    <p className="text-xs text-muted-foreground">{insightText("aliasHelp", { example: '[{"name":"Alice","email":"alice@example.com","nickname":"小艾"}]' })}</p>
    {error && <p role="alert" className="text-sm text-destructive">{insightText(error)}</p>}
  </AutoField>;
}
