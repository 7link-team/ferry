import { useEffect,useRef,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { base,operationResource } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { executionText,useCommonTranslation } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { artifactDiff,deploymentActive,deploymentState,type Deployment,type Preview,type Revision } from "./deployment";

export function DeploymentDetail({ navigation, workspace, deploymentId, revisionId, permissions }: { navigation: Navigation;
  workspace: string; deploymentId?: string; revisionId?: string; permissions: Set<string>;
}) {
  useCommonTranslation();
  const stateLabel = deploymentState;
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
      const result = await api<Preview & { artifact: string }>(`${root}/${app.id}/${operationResource(verb)}`, { method: "POST", body: JSON.stringify(body), signal: controller.signal });
      if (controller.signal.aborted) return;
      if (verb === "preview") { setPreview(result); setRendered(undefined); }
      else if (verb === "render") setRendered(result.artifact);
      else { setPreview(undefined); setReload((n) => n + 1); }
    } catch (e) { if (!controller.signal.aborted) setError(errorText(e)); }
    finally { if (!controller.signal.aborted) setBusy(false); }
  };
  const execute = (rollback: boolean) => {
    if (!app || !preview || preview.version !== app.version) return;
    if (!confirm(rollback ? executionText("deployment.rollbackConfirm", { number: revision?.number }) : executionText("deployment.applyConfirm"))) return;
    void action(rollback ? "rollback" : "apply", { version: app.version, expected_hash: preview.expected_hash, preview_token: preview.preview_token, ...(preview.target_hashes ? { target_hashes: preview.target_hashes } : {}), ...(rollback ? { revision_id: revision?.id } : {}) });
  };
  if (!app) return <div role={error ? "alert" : "status"}>{error || executionText("deployment.loading")}{error && <Button onClick={() => setReload((n) => n + 1)}>{executionText("shared.common.common.retry")}</Button>}</div>;
  const blocked = busy || deploymentActive(app.state) || app.state === "needs-review";
  return <div className="min-h-0 overflow-auto space-y-5 pb-6">
    <header className="flex flex-wrap items-center gap-3 border-b pb-4">
      <h2 className="text-lg font-semibold">{app.name}</h2>{deploymentState(app.state)}
      <span className="text-sm text-muted-foreground">{executionText("deployment.composeTarget", { id: app.target_ref })}</span>
    </header>
    <div className="flex flex-wrap gap-2" aria-busy={busy}>
      <Button variant="outline" disabled={busy} onClick={() => void action("render")}>{executionText("deployment.viewRendered")}</Button>
      <Button variant="outline" disabled={blocked} onClick={() => void action("preview", revisionId ? { revision_id: revisionId } : {})}>{executionText("deployment.previewDiff")}</Button>
      {permissions.has("deployments.apply") && !revisionId && <Button disabled={blocked || !preview || preview.version !== app.version} onClick={() => execute(false)}>{executionText("deployment.apply")}</Button>}
      {revisionId && permissions.has("deployments.rollback") && revision?.state === "succeeded" && <Button variant="outline" disabled={blocked || !preview || preview.version !== app.version} onClick={() => execute(true)}>{executionText("deployment.rollbackNumber", { number: revision.number })}</Button>}
      <Button variant="outline" disabled={busy} onClick={() => setReload((n) => n + 1)}>{executionText("deployment.refresh")}</Button>
      <Button variant="outline" onClick={() => navigation.navigate({ item: "revisions", mode: "list" }, { deployment_id: app.id })}>{executionText("shared.common.nav.deploymentHistory")}</Button>
      {permissions.has("deployments.update") && <Button variant="outline" disabled={blocked} onClick={() => navigation.navigate({ item: "apps", id: app.id, mode: "edit" }, {})}>{executionText("deployment.editApp")}</Button>}
    </div>
    <p className="text-sm text-muted-foreground">{executionText("deployment.applyHelp")}</p>
    {error && <p role="alert" className="text-destructive">{error}</p>}
    {app.state === "needs-review" && <section className="space-y-3 border-l-2 border-destructive pl-4">
      <h3 className="font-semibold">{executionText("deployment.reviewRequired")}</h3><p>{executionText("deployment.reviewHelp")}</p>
      {permissions.has("deployments.apply") && <><AutoField label={executionText("deployment.reviewNote")}>
        <textarea id="review-note" value={note} maxLength={500} onChange={(e) => setNote(e.target.value)} className="w-full rounded border bg-background p-3" />
        </AutoField><Button disabled={busy || !note.trim()} onClick={() => { if (confirm(executionText("deployment.acknowledgeConfirm"))) void action("acknowledge", { version: app.version, note }); }}>{executionText("deployment.acknowledge")}</Button></>}
    </section>}
    {preview && <section className="space-y-3"><h3 className="font-semibold">{executionText("deployment.artifactDiff")}</h3><p className="text-sm text-muted-foreground">{preview.notice}</p>
      {!!preview.variables?.length && <section aria-label={executionText("deployment.variableReferences")} className="space-y-2 rounded border p-3">
        <h4 className="text-sm font-medium">{executionText("deployment.variableReferencesPrefix")} {revisionId ? executionText("deployment.historicalRevision") : executionText("deployment.pinnedAtQueue")}</h4>
        <ul className="space-y-1 text-sm">{preview.variables.map(v => <li key={`${v.service}/${v.environment ?? v.path}/${v.kind ?? "env"}/${v.name ?? "file"}`}>
          {v.kind === "file" ? <span>{executionText("deployment.configFileReference", { service: v.service, path: v.path, version: v.template_version })}</span> : <><span className="font-mono">{v.service}.{v.environment ?? v.path}</span> ← {v.name} · {v.scope_ref !== "0" ? executionText("deployment.appScope") : v.scope === "global" ? executionText("shared.resource.variable.global") : executionText("deployment.moduleScope")} · v{v.version} · {v.secret ? executionText("shared.resource.variable.secret") : executionText("run.regularVariable")}</>}
        </li>)}</ul>
        <p className="text-xs text-muted-foreground">{executionText("deployment.variableHistoryHelp")}</p>
      </section>}
      <pre className="max-h-96 overflow-auto rounded border bg-muted/30 p-4 text-xs">{artifactDiff(preview.current, revisionId && revision?.artifact ? revision.artifact : preview.desired)}</pre>
      <details><summary className="cursor-pointer text-sm">{executionText("deployment.fullDesired")}</summary><pre className="overflow-auto p-4 text-xs">{revisionId && revision?.artifact ? revision.artifact : preview.desired}</pre></details>
    </section>}
    {rendered !== undefined && <section><h3 className="mb-2 font-semibold">{executionText("deployment.currentRendered")}</h3><pre className="max-h-96 overflow-auto rounded border p-4 text-xs">{rendered}</pre></section>}
    {!!app.current_targets?.length && <details><summary className="cursor-pointer text-sm">{executionText("deployment.hostRevisions")}</summary><ul className="mt-2 space-y-1 text-sm">{app.current_targets.map((target) => <li key={target.host_id}>{executionText("deployment.hostRevision", { id: target.host_id, revision: target.current_revision })}</li>)}</ul></details>}
    {revision && <section className="space-y-3"><h3 className="font-semibold">{executionText("deployment.revisionTitle", { number: revision.number, action: revision.action === "rollback" ? executionText("shared.resource.deployment.rollback") : executionText("shared.resource.variable.deploy"), state: stateLabel(revision.state) })}</h3>
      <pre aria-label={executionText("shared.resource.cron.output")} className="max-h-80 overflow-auto whitespace-pre-wrap rounded border bg-muted/30 p-4 text-xs">{revision.output || executionText("deployment.resultPending")}</pre>
      {!!revision.targets?.length && <section aria-label={executionText("deployment.perHostResults")} className="space-y-2"><h4 className="text-sm font-medium">{executionText("deployment.sequentialHosts")}</h4><ol className="space-y-2">{revision.targets.map((target) => <li key={target.id} className="rounded border p-3"><details><summary className="cursor-pointer text-sm">{executionText("deployment.hostNumber", { id: target.host_id })} · {deploymentState(target.state)}</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap text-xs">{target.output || executionText("deployment.waiting")}</pre></details></li>)}</ol></section>}
      <details><summary className="cursor-pointer text-sm">{executionText("deployment.savedArtifact")}</summary><pre className="overflow-auto p-4 text-xs">{revision.artifact || executionText("deployment.artifactPending")}</pre></details>
    </section>}
  </div>;
}
