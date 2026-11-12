import { useState } from "react";
import { commonText, useCommonTranslation } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { BlueprintWizard } from "./BlueprintWizard";
import { readSpec } from "./blueprint-spec";
import type { Deployment } from "./deployment";

const w = (key: string, values?: Record<string, unknown>) => commonText(`resource-domain:wizard.${key}`, values);

/** Entry to edit a spec-managed deployment's `deploy_spec` in the blueprint wizard. */
export function BlueprintSpecEdit({ workspace, deployment, permissions, navigation, disabled = false, onSaved }: {
  workspace: string; deployment: Deployment; permissions: Set<string>; navigation: Navigation; disabled?: boolean; onSaved: () => void;
}) {
  useCommonTranslation();
  // The detail page keeps polling; the wizard edits the deployment as it was when opened.
  const [snapshot, setSnapshot] = useState<Deployment>();
  if (!deployment.deploy_spec || !permissions.has("deployments.update")) return null;
  if (!snapshot) return <div><Button type="button" variant="outline" className="min-h-11 lg:min-h-9" disabled={disabled} onClick={() => setSnapshot(deployment)}>{w("editSpec")}</Button></div>;
  const parsed = readSpec(snapshot.deploy_spec ?? "");
  return <section aria-label={w("editTitle")} className="min-w-0 rounded-md border p-3 sm:p-4">
    {parsed.spec
      ? <BlueprintWizard workspace={workspace} templateId={parsed.spec.template.id} version={parsed.spec.template.version} permissions={permissions} navigation={navigation}
        deployment={snapshot} onCancel={() => setSnapshot(undefined)} onDone={() => { setSnapshot(undefined); onSaved(); }} />
      : <div role="alert" className="space-y-2 text-sm text-destructive">
        <p>{w("specUnreadable")}</p>
        {parsed.errors.map((error, index) => <p key={index} className="break-words">{error}</p>)}
        <Button type="button" variant="outline" className="min-h-11 lg:min-h-9" onClick={() => setSnapshot(undefined)}>{w("cancel")}</Button>
      </div>}
  </section>;
}
