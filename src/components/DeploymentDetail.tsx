import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../api";
import { base } from "../resource";
import { navigate } from "../router";
import { artifactDiff, deploymentActive, deploymentState, type Deployment, type Preview, type Revision } from "../deployment";
import { Button } from "./ui/button";
import { Badge } from "./ui/badge";

export function DeploymentDetail({ workspace, deploymentId, revisionId, permissions }: {
  workspace: string; deploymentId?: string; revisionId?: string; permissions: Set<string>;
}) {
  const [app, setApp] = useState<Deployment>();
  const [revision, setRevision] = useState<Revision>();
  const [preview, setPreview] = useState<Preview>();
  const [rendered, setRendered] = useState<string>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [reload, setReload] = useState(0);
  const [note, setNote] = useState("");
  const actionRequest = useRef<AbortController | null>(null);
  const appRef = useRef(app); appRef.current = app;
  const root = base(workspace, "deployments");
  useEffect(() => {
    let live = true, finished = false;
    let request: AbortController | undefined, timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      if (!live || document.hidden || request || finished) return;
      const controller = new AbortController(); request = controller;
      try {
        const selected = revisionId ? await api<Revision>(`${base(workspace, "deployment-revisions")}/${encodeURIComponent(revisionId)}`, { signal: controller.signal }) : undefined;
        const id = selected?.deployment_id ?? deploymentId;
        const next = await api<Deployment>(`${root}/${encodeURIComponent(id ?? "")}`, { signal: controller.signal });
        if (!live || controller.signal.aborted) return;
        setApp(next); setRevision(selected ?? next.latest_revision); setError("");
        finished = !deploymentActive(next.state);
        if (appRef.current && appRef.current.version !== next.version) setPreview(undefined);
      } catch (e) { if (live && !controller.signal.aborted) setError(errorText(e)); }
      finally { request = undefined; if (live && !finished && !document.hidden) timer = setTimeout(load, 1200); }
    };
    const visibility = () => {
      clearTimeout(timer);
      if (document.hidden) request?.abort(); else { finished = false; void load(); }
    };
    void load(); document.addEventListener("visibilitychange", visibility);
    return () => { live = false; clearTimeout(timer); request?.abort(); document.removeEventListener("visibilitychange", visibility); };
  }, [root, workspace, deploymentId, revisionId, reload]);
  useEffect(() => () => actionRequest.current?.abort(), []);

  const action = async (verb: string, body: unknown = {}) => {
    if (!app || busy) return;
    setBusy(true); setError("");
    const controller = new AbortController(); actionRequest.current = controller;
    try {
      const result = await api<Preview & { artifact: string }>(`${root}/${app.id}/actions/${verb}`, { method: "POST", body: JSON.stringify(body), signal: controller.signal });
      if (controller.signal.aborted) return;
      if (verb === "preview") { setPreview(result); setRendered(undefined); }
      else if (verb === "render") setRendered(result.artifact);
      else { setPreview(undefined); setReload((n) => n + 1); }
    } catch (e) { if (!controller.signal.aborted) setError(errorText(e)); }
    finally { if (!controller.signal.aborted) setBusy(false); }
  };
  const execute = (rollback: boolean) => {
    if (!app || !preview || preview.version !== app.version) return;
    if (!confirm(rollback ? `回滚到 revision #${revision?.number}？将恢复当时的变量值，旧密钥可能已经失效；卷数据不会回滚。` : "执行当前部署？失败不会自动回滚。")) return;
    void action(rollback ? "rollback" : "apply", { version: app.version, expected_hash: preview.expected_hash, preview_token: preview.preview_token, ...(preview.target_hashes ? { target_hashes: preview.target_hashes } : {}), ...(rollback ? { revision_id: revision?.id } : {}) });
  };
  if (!app) return <div role={error ? "alert" : "status"}>{error || "正在读取部署…"}{error && <Button onClick={() => setReload((n) => n + 1)}>重试</Button>}</div>;
  const blocked = busy || deploymentActive(app.state) || app.state === "needs-review";
  return <div className="min-h-0 overflow-auto space-y-5 pb-6">
    <header className="flex flex-wrap items-center gap-3 border-b pb-4">
      <h2 className="text-lg font-semibold">{app.name}</h2><Badge variant="outline">{deploymentState(app.state)}</Badge>
      <span className="text-sm text-muted-foreground">Compose · 主机 {app.target_ref}</span>
    </header>
    <div className="flex flex-wrap gap-2" aria-busy={busy}>
      <Button variant="outline" disabled={busy} onClick={() => void action("render")}>查看渲染产物</Button>
      <Button variant="outline" disabled={blocked} onClick={() => void action("preview", revisionId ? { revision_id: revisionId } : {})}>预览目标差异</Button>
      {permissions.has("deployments.apply") && !revisionId && <Button disabled={blocked || !preview || preview.version !== app.version} onClick={() => execute(false)}>执行部署</Button>}
      {revisionId && permissions.has("deployments.rollback") && revision?.state === "succeeded" && <Button variant="outline" disabled={blocked || !preview || preview.version !== app.version} onClick={() => execute(true)}>回滚到 #{revision.number}</Button>}
      <Button variant="outline" disabled={busy} onClick={() => setReload((n) => n + 1)}>刷新状态</Button>
      <Button variant="outline" onClick={() => navigate({ item: "revisions", mode: "list" }, { deployment_id: app.id })}>部署历史</Button>
      {permissions.has("deployments.update") && <Button variant="outline" disabled={blocked} onClick={() => navigate({ item: "apps", id: app.id, mode: "edit" }, {})}>编辑应用</Button>}
    </div>
    <p className="text-sm text-muted-foreground">先预览再执行。成功表示容器已运行（有健康检查时需通过）；不包含业务功能验收。卷数据不会随版本回滚。</p>
    {error && <p role="alert" className="text-destructive">{error}</p>}
    {app.state === "needs-review" && <section className="space-y-3 border-l-2 border-destructive pl-4">
      <h3 className="font-semibold">执行结果需要核实</h3><p>远端命令可能仍在运行。请核实进程、容器及目标产物，确认后才能再次部署。</p>
      {permissions.has("deployments.apply") && <><label htmlFor="review-note" className="block text-sm">核实说明</label>
        <textarea id="review-note" value={note} maxLength={500} onChange={(e) => setNote(e.target.value)} className="w-full rounded border bg-background p-3" />
        <Button disabled={busy || !note.trim()} onClick={() => { if (confirm("已核实远端执行情况，允许后续重新预览和部署？")) void action("acknowledge", { version: app.version, note }); }}>已核实，解除阻止</Button></>}
    </section>}
    {preview && <section className="space-y-3"><h3 className="font-semibold">产物差异</h3><p className="text-sm text-muted-foreground">{preview.notice}</p>
      {!!preview.variables?.length && <section aria-label="变量引用" className="space-y-2 rounded border p-3">
        <h4 className="text-sm font-medium">变量引用 · {revisionId ? "历史版本" : "入队时固定"}</h4>
        <ul className="space-y-1 text-sm">{preview.variables.map(v => <li key={`${v.service}/${v.environment ?? v.path}/${v.kind ?? "env"}/${v.name ?? "file"}`}>
          {v.kind === "file" ? <span>配置文件 {v.service}:{v.path} · 模板 v{v.template_version}</span> : <><span className="font-mono">{v.service}.{v.environment ?? v.path}</span> ← {v.name} · {v.scope_ref !== "0" ? "应用专属" : v.scope === "global" ? "全局" : "部署模块"} · v{v.version} · {v.secret ? "密钥" : "普通变量"}</>}
        </li>)}</ul>
        <p className="text-xs text-muted-foreground">变量值加密保存于部署历史，更新当前变量不会删除历史值。预览和执行日志不显示变量值。</p>
      </section>}
      <pre className="max-h-96 overflow-auto rounded border bg-muted/30 p-4 text-xs">{artifactDiff(preview.current, revisionId && revision?.artifact ? revision.artifact : preview.desired)}</pre>
      <details><summary className="cursor-pointer text-sm">查看完整待执行产物</summary><pre className="overflow-auto p-4 text-xs">{revisionId && revision?.artifact ? revision.artifact : preview.desired}</pre></details>
    </section>}
    {rendered !== undefined && <section><h3 className="mb-2 font-semibold">当前 manifest 渲染产物</h3><pre className="max-h-96 overflow-auto rounded border p-4 text-xs">{rendered}</pre></section>}
    {!!app.current_targets?.length && <details><summary className="cursor-pointer text-sm">各主机当前成功版本</summary><ul className="mt-2 space-y-1 text-sm">{app.current_targets.map((target) => <li key={target.host_id}>主机 {target.host_id} · revision {target.current_revision}</li>)}</ul></details>}
    {revision && <section className="space-y-3"><h3 className="font-semibold">Revision #{revision.number} · {revision.action === "rollback" ? "回滚" : "部署"} · {deploymentState(revision.state)}</h3>
      <pre aria-label="执行输出" className="max-h-80 overflow-auto whitespace-pre-wrap rounded border bg-muted/30 p-4 text-xs">{revision.output || "执行结果尚未返回。"}</pre>
      {!!revision.targets?.length && <section aria-label="逐台执行结果" className="space-y-2"><h4 className="text-sm font-medium">目标主机 · 按顺序执行</h4><ol className="space-y-2">{revision.targets.map((target) => <li key={target.id} className="rounded border p-3"><details><summary className="cursor-pointer text-sm">主机 {target.host_id} · {deploymentState(target.state)}</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap text-xs">{target.output || "等待执行。"}</pre></details></li>)}</ol></section>}
      <details><summary className="cursor-pointer text-sm">保存的执行产物（版本摘要已固定）</summary><pre className="overflow-auto p-4 text-xs">{revision.artifact || "尚未生成执行产物。"}</pre></details>
    </section>}
  </div>;
}
