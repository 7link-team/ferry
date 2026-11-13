import { useMemo, type ComponentProps } from "react";
import * as feature from "../../features/deployments/DeploymentDetail";
import { monitoringForms } from "../../features/monitoring/resource-config";
import { ResourceForm } from "../resources/ResourceForm";
import { href,navigate } from "../router";
import { applicationSections } from "../../features/deployments/application-sections";
import { readSpec } from "../../features/deployments/blueprint-spec";
import { commonText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import { form, type AnyRow } from "../../shared/resources/config";
export * from "../../features/deployments/DeploymentDetail";
const navigation = { href, navigate };
export function DeploymentDetail(props: Omit<ComponentProps<typeof feature.DeploymentDetail>, "navigation" | "healthContent" | "renderConfiguration">) {
  const healthForm = useMemo(() => monitoringForms({ workspace: props.workspace, permissions: props.permissions, ...navigation })["application-health"]!, [props.workspace, props.permissions]);
  const health = props.deploymentId && props.permissions.has("alerts.read") ? <ResourceForm config={healthForm} workspace={props.workspace} permissions={props.permissions} showHeading={false} showViewNavigation={false} /> : undefined;
  return <div className="min-h-0 min-w-0 overflow-auto"><feature.DeploymentDetail {...props} navigation={navigation} healthContent={health}
    renderConfiguration={(app, runtime) => {
      const values = app.deploy_spec ? readSpec(app.deploy_spec).spec?.values : undefined;
      const record: AnyRow = { ...app, ...(values ? { values } : {}) };
      const config = form<AnyRow>({ resource: "deployments", width: "full", sectionNavigation: "scroll", sectionAppearance: "configuration", heading: row => String(row.name ?? ""), blank: {},
        readOnly: row => row.target_kind === "blueprint",
        sections: applicationSections([{ fields: [
          { key: "name", label: "Name", labelKey: "common-domain:common.name", type: "text" },
          { key: "description", label: "Description", labelKey: "common-domain:common.description", type: "textarea", span: 2 },
          { key: "version", label: "Version", labelKey: "common-domain:common.version", type: "number" },
          { key: "updated_at", label: "Updated", labelKey: "resource-domain:platform.workspaceUpdated", type: "text", display: row => row.updated_at ? new Date(Number(row.updated_at)).toLocaleString(currentLocale()) : "—" },
          { key: "mode", label: "Driver", labelKey: "resource-domain:deployment.mode", type: "text" },
          { key: "target_ref", label: "Target", labelKey: "resource-domain:resource.target", type: "text", display: row => String(row.target_name || (row.target_kind === "blueprint" ? commonText("resource-domain:wizard.roles") : row.target_ref) || "—") },
          { key: "workdir", label: "Working directory", labelKey: "resource-domain:deployment.workdir", type: "text" },
          { key: "manifest", label: "Definition", labelKey: "resource-domain:template.definition", type: "custom", when: row => !!row.manifest, span: 2 },
          { key: "values", label: "Parameters", labelKey: "resource-domain:template.parameters", type: "custom", span: 2 },
          { key: "files", label: "Files", labelKey: "resource-domain:market.files", type: "custom", span: 2 },
          { key: "runtime", label: "Runtime", labelKey: "resource-domain:application.runtime", type: "custom", span: 2, render: () => runtime },
        ] }], props.workspace),
      });
      return <ResourceForm config={config} record={record} workspace={props.workspace} permissions={props.permissions} />;
    }} /></div>;
}
