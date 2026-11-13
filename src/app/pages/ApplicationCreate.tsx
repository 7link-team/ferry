import { useCallback, useEffect, useState } from "react";
import { ResourceForm } from "../resources/ResourceForm";
import { api, errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { FormConfig } from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { BlueprintWizard } from "../../features/deployments/BlueprintWizard";
import { isBlueprint } from "../../features/deployments/blueprint-spec";

/** One creation entry. Classify the selected definition before choosing its controls;
 * a Blueprint must never be rendered into an ordinary Application's services field. */
export function ApplicationCreate({ workspace, permissions, config, navigation }: {
  workspace: string; permissions: Set<string>; config: FormConfig<Record<string, unknown>>; navigation: Navigation;
}) {
  const query = new URLSearchParams(location.search);
  const templateId = query.get("template") ?? "";
  const requestedVersion = Number(query.get("version")) || 0;
  const [blueprint, setBlueprint] = useState<{ id: string; version: number }>();
  const [loading, setLoading] = useState(!!templateId);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [generation, setGeneration] = useState(0);
  const classify = useCallback(async (id: string, signal?: AbortSignal) => {
    const row = await api<{ content: string; version: number }>(`${base(workspace, "app-templates")}/${encodeURIComponent(id)}`, signal ? { signal } : {});
    return isBlueprint(row.content) ? { id, version: row.version } : undefined;
  }, [workspace]);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(!!templateId); setError(""); setBlueprint(undefined);
    if (templateId) void classify(templateId, controller.signal).then(value => {
      if (!controller.signal.aborted) setBlueprint(value ? { ...value, version: requestedVersion || value.version } : undefined);
    }).catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [templateId, requestedVersion, classify, attempt]);
  const select = async (id: string) => {
    const selected = await classify(id);
    if (!selected) return false;
    setBlueprint(selected);
    const url = new URL(location.href);
    url.search = new URLSearchParams({ template: id, version: String(selected.version) }).toString();
    history.replaceState(null, "", url);
    return true;
  };
  const changeTemplate = () => {
    history.replaceState(null, "", location.pathname);
    setBlueprint(undefined); setGeneration(value => value + 1);
  };
  if (loading) return <LoadingSkeleton variant="page" />;
  if (error) return <div role="alert" className="space-y-3"><p>{error}</p><Button type="button" onClick={() => setAttempt(value => value + 1)}>{commonText("common.retry")}</Button></div>;
  return <div data-application-create className="min-w-0">
    {blueprint ? <>
      <div className="mb-3 flex flex-wrap gap-2"><Button type="button" variant="outline" className="min-h-11" onClick={changeTemplate}>{commonText("form.changeTemplate")}</Button></div>
      <BlueprintWizard key={`${blueprint.id}/${blueprint.version}`} workspace={workspace} templateId={blueprint.id} version={blueprint.version}
        permissions={permissions} navigation={navigation} onCancel={() => navigation.navigate({ item: "apps", mode: "list" }, {})} />
    </> : <ResourceForm key={generation} config={config} workspace={workspace} permissions={permissions}
      createInPlace onSaved={id => { if (id) navigation.navigate({ group: "deploy", item: "apps", id, mode: "view" }, {}); }}
      templatePresentation="page" onSelectTemplate={select} />}
  </div>;
}
