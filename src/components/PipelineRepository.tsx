import { useEffect, useState } from "react";
import { RepositorySource } from "./RepositorySource";
import { Button } from "./ui/button";
import type { Option } from "./dsl";

/** Adapt the shared repository connection input to a pipeline's atomic source binding. */
export function PipelineRepository({ workspace, draft, options, readOnly, onChange, onValidityChange }: {
  workspace: string; draft: Record<string, unknown>; options: Option[]; readOnly: boolean;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (blocked: boolean) => void;
}) {
  const [mode, setMode] = useState(draft.repository_selection ? "connection" : "saved");
  const [source, setSource] = useState<Record<string, unknown>>(
    (draft.repository_selection as Record<string, unknown>) ?? { provider: "", name: "", url: "", credential_id: "0" },
  );
  const [error, setError] = useState("");
  useEffect(() => {
    if (mode !== "connection") onValidityChange(false);
  }, [mode, onValidityChange]);
  const selectClass = "h-10 w-full rounded-md border border-input bg-background px-3 text-sm";
  function checkout(): string | undefined {
    try {
      const definition = JSON.parse(String(draft.definition));
      if (!Array.isArray(definition.steps)) throw new Error();
      if (!definition.steps.some((step: { checkout?: boolean }) => step.checkout))
        definition.steps.unshift({ name: "拉取代码", checkout: true, timeout_seconds: 1800 });
      setError("");
      return JSON.stringify(definition, null, 2);
    } catch { setError("请先修正流水线定义，再添加拉取代码步骤。"); return undefined; }
  }
  if (readOnly) return <p className="text-sm">仓库：{options.find(row => row.value === draft.repository_id)?.label || "未选择"}</p>;
  return <div className="space-y-4">
    <label className="block space-y-2"><span className="text-sm font-medium">构建代码来源</span>
      <select aria-label="构建代码来源" className={selectClass} value={mode} onChange={event => {
        const next = event.target.value; setMode(next); setError("");
        onChange({ repository_selection: null, repository_id: "" });
        setSource({ provider: "", name: "", url: "", credential_id: "0" });
      }}><option value="saved">已保存仓库 / 无代码来源</option><option value="connection">通过平台连接选择仓库</option></select>
    </label>
    {mode === "saved" ? <label className="block space-y-2"><span className="text-sm font-medium">仓库</span>
      <select aria-label="仓库" className={selectClass} value={String(draft.repository_id ?? "")} onChange={event => onChange({ repository_id: event.target.value, repository_selection: null })}>
        <option value="">不选择（仅执行命令）</option>{options.map(row => <option key={row.value} value={row.value}>{row.label}</option>)}
      </select>
    </label> : <RepositorySource platformOnly workspace={workspace} draft={source} readOnly={false} onValidityChange={onValidityChange} onChange={patch => {
      const selected = { ...source, ...patch };
      // This is the local display name; keep the full remote identity in its URL.
      selected.name = [...String(selected.name ?? "")].slice(0, 60).join("");
      setSource(selected);
      const definition = selected.url ? checkout() : undefined;
      onChange({ repository_id: "", repository_selection: selected, ...(definition ? { definition } : {}) });
    }} />}
    {(draft.repository_id || source.url) ? <Button type="button" variant="outline" onClick={() => {
      const definition = checkout(); if (definition) onChange({ definition });
    }}>添加拉取代码步骤</Button> : null}
    <p className="text-xs text-muted-foreground">平台连接选择仓库后会添加“拉取代码”步骤，再执行你配置的构建命令。已保存流水线可手动添加；已有手写 Git 命令不受影响。</p>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}
