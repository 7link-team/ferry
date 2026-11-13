import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { Tabs } from "radix-ui";
import type { ReactNode } from "react";
import { useQueryValue } from "../../shared/routing/url-state";
import { lazy,Suspense,useEffect,useId,useRef,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { base,named,operationResource } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText,executionText,useCommonTranslation } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { Badge } from "../../shared/ui/badge";
import { deploymentActive,deploymentState,revisionActionLabel,type Deployment,type Preview,type Revision } from "./deployment";
import { BlueprintSpecEdit } from "./BlueprintSpecEdit";
import { BlueprintTarget, BlueprintTopology } from "./BlueprintTopology";
import { FailoverAttention, FailoverTimeline } from "./FailoverTimeline";

const CodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));

export function DeploymentDetail({ navigation, workspace, deploymentId, revisionId, permissions, healthContent, renderConfiguration }: { navigation: Navigation;
  workspace: string; deploymentId?: string; revisionId?: string; permissions: Set<string>; healthContent?: ReactNode;
  renderConfiguration?: (deployment: Deployment, runtime: ReactNode) => ReactNode;
}) {
  useCommonTranslation();
  const editorId = useId();
  const [tab, setTab] = useQueryValue("tab", "info");
  const healthAvailable = !revisionId && permissions.has("alerts.read") && !!healthContent;
  const activeTab = healthAvailable && tab === "health" ? "health" : "info";
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
    let interval = 1200;
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
        // Historical content is fixed, but action availability follows the live application.
        finished = !deploymentActive(next.state) && !deploymentActive(selected?.state ?? next.state) && (!!revisionId || next.target_kind !== "blueprint");
        interval = deploymentActive(next.state) || deploymentActive(selected?.state ?? "") ? 1200 : 10_000;
        if (appRef.current && appRef.current.version !== next.version) setPreview(undefined);
      } catch (e) { if (live && !controller.signal.aborted) setError(errorText(e)); }
      finally { request = undefined; if (live && !finished && !document.hidden) timer = setTimeout(load, interval); }
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
  if (!app && !error) return <LoadingSkeleton variant="page" label={executionText("deployment.loading")} />;
  if (!app) return <div role={error ? "alert" : "status"}>{error || executionText("deployment.loading")}{error && <Button onClick={() => setReload((n) => n + 1)}>{executionText("shared.common.common.retry")}</Button>}</div>;
  const blocked = busy || deploymentActive(app.state) || app.state === "needs-review";
  const body = <div className="min-h-0 min-w-0 space-y-5 pb-6">
    <header className="flex min-w-0 flex-wrap items-center gap-3 border-b bg-background py-3">
      {revisionId && <h2 className="break-words text-lg font-semibold">{app.name}</h2>}<Badge variant={["failed", "needs-review"].includes(app.state) ? "destructive" : "secondary"}>{deploymentState(app.state)}</Badge>
      <span className="text-sm text-muted-foreground">{commonText("resource-domain:resource.target")} · {app.target_kind === "blueprint" ? <BlueprintTarget workspace={workspace} deployment={app} permissions={permissions} /> : named(app.target_name, app.target_ref)}</span>
    </header>
    <Tabs.Root value={activeTab} onValueChange={setTab} className="min-w-0 space-y-4">
      {healthAvailable && <Tabs.List aria-label={app.name} className="flex flex-wrap gap-2 border-b">
        {([['info', commonText("common.basic")], ['health', commonText("monitoring:healthTab")]] as const).map(([value, label]) => <Tabs.Trigger key={value} value={value} className="min-h-11 border-b-2 border-transparent px-3 py-2 text-sm data-[state=active]:border-primary data-[state=active]:text-primary">{label}</Tabs.Trigger>)}
      </Tabs.List>}
      <Tabs.Content value="health" className="min-w-0">{healthAvailable && healthContent}</Tabs.Content>
      <Tabs.Content value="info" forceMount className="space-y-5 data-[state=inactive]:hidden">
    {!revisionId && app.target_kind === "blueprint" && <FailoverAttention workspace={workspace} deployment={app} permissions={permissions} onDone={() => setReload(n => n + 1)} />}
    <div className="flex flex-wrap gap-2" aria-busy={busy}>
      <Button variant="outline" disabled={busy} onClick={() => revisionId ? setRendered(revision?.rendered ?? "") : void action("render")}>{executionText("deployment.viewRendered")}</Button>
      <Button variant="outline" disabled={blocked} onClick={() => void action("preview", revisionId ? { revision_id: revisionId } : {})}>{executionText("deployment.previewDiff")}</Button>
      {permissions.has("deployments.apply") && !revisionId && <Button disabled={blocked || !preview || preview.version !== app.version} onClick={() => execute(false)}>{executionText("deployment.apply")}</Button>}
      {revisionId && permissions.has("deployments.rollback") && revision?.state === "succeeded" && <Button variant="outline" disabled={blocked || !preview || preview.version !== app.version} onClick={() => execute(true)}>{executionText("deployment.rollbackNumber", { number: revision.number })}</Button>}
      <Button variant="outline" disabled={busy} onClick={() => setReload((n) => n + 1)}>{executionText("deployment.refresh")}</Button>
      <Button variant="outline" onClick={() => navigation.navigate({ item: "revisions", mode: "list" }, { deployment_id: app.id })}>{executionText("shared.common.nav.deploymentHistory")}</Button>
    </div>
    <p className="text-sm text-muted-foreground">{executionText("deployment.applyHelp")}</p>
    {error && <p role="alert" className="text-destructive">{error}</p>}
    {!revisionId && <BlueprintSpecEdit workspace={workspace} deployment={app} permissions={permissions} navigation={navigation} disabled={blocked} onSaved={() => setReload((n) => n + 1)} />}
    {!revisionId && app.target_kind === "blueprint" && <BlueprintTopology workspace={workspace} deployment={app} permissions={permissions} onDone={() => setReload(n => n + 1)} />}
    {!revisionId && app.target_kind === "blueprint" && <FailoverTimeline key={`${workspace}/${app.id}`} workspace={workspace} deployment={app} />}
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
      {preview.current === (revisionId && revision?.artifact ? revision.artifact : preview.desired)
        ? <p className="rounded-md border p-4 text-sm text-muted-foreground">{commonText("artifact.identical")}</p>
        : <div className="h-96 min-w-0 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" />}><CodeEditor originalValue={preview.current} value={revisionId && revision?.artifact ? revision.artifact : preview.desired} language="yaml" path={`deployment-${editorId}-diff.yaml`} label={executionText("deployment.artifactDiff")} readOnly onChange={() => {}} /></Suspense></div>}
      <details><summary className="cursor-pointer text-sm">{executionText("deployment.fullDesired")}</summary><pre className="overflow-auto p-4 text-xs">{revisionId && revision?.artifact ? revision.artifact : preview.desired}</pre></details>
    </section>}
    {rendered !== undefined && <section><h3 className="mb-2 font-semibold">{executionText(revisionId ? "deployment.historicalRevision" : "deployment.currentRendered")}</h3><div className="h-96 min-w-0 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" />}><CodeEditor value={rendered} language="yaml" path={`deployment-${editorId}-rendered.yaml`} label={executionText("deployment.viewRendered")} readOnly onChange={() => {}} /></Suspense></div></section>}
    {!!app.current_targets?.length && <details><summary className="cursor-pointer text-sm">{executionText("deployment.hostRevisions")}</summary><ul className="mt-2 space-y-1 text-sm">{app.current_targets.map((target) => <li key={target.host_id}>{executionText("deployment.hostRevision", { id: target.host_id, revision: target.current_revision })}</li>)}</ul></details>}
    {revision && <section className="space-y-3"><h3 className="font-semibold">{executionText("deployment.revisionTitle", { number: revision.number, action: revisionActionLabel(revision.action), state: stateLabel(revision.state) })}</h3>
      <dl className="grid gap-3 text-sm sm:grid-cols-3">{[
        [commonText("resource-domain:execution.triggeredBy"), named(revision.triggered_by_name, revision.triggered_by)],
        [commonText("resource-domain:execution.startedAt"), revision.started_at ? new Date(revision.started_at).toLocaleString(currentLocale()) : "—"],
        [commonText("resource-domain:deployment.finishedAt"), revision.finished_at ? new Date(revision.finished_at).toLocaleString(currentLocale()) : "—"],
      ].map(([label, value]) => <div key={label} className="min-w-0"><dt className="text-xs text-muted-foreground">{label}</dt><dd className="mt-1 break-all">{value}</dd></div>)}</dl>
      <pre aria-label={executionText("shared.resource.cron.output")} className="max-h-80 overflow-auto whitespace-pre-wrap rounded border bg-muted/30 p-4 text-xs">{revision.output || executionText("deployment.resultPending")}</pre>
      {!!revision.targets?.length && <section aria-label={executionText("deployment.perHostResults")} className="space-y-2"><h4 className="text-sm font-medium">{executionText("deployment.sequentialHosts")}</h4><ol className="space-y-2">{revision.targets.map((target) => <li key={target.id} className="rounded border p-3"><details><summary className="cursor-pointer text-sm">{executionText("deployment.hostNumber", { id: target.host_id })} · {deploymentState(target.state)}</summary><pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap text-xs">{target.output || executionText("deployment.waiting")}</pre></details></li>)}</ol></section>}
      <details><summary className="cursor-pointer text-sm">{executionText("deployment.savedArtifact")}</summary><pre className="overflow-auto p-4 text-xs">{revision.artifact || executionText("deployment.artifactPending")}</pre></details>
    </section>}
      </Tabs.Content>
    </Tabs.Root>
  </div>;
  if (revisionId) return body;
  return renderConfiguration ? renderConfiguration(app, body) : body;

}
