import { useMemo, type ComponentProps } from "react";
import * as feature from "../../features/deployments/DeploymentDetail";
import { monitoringForms } from "../../features/monitoring/resource-config";
import { ResourceForm } from "../resources/ResourceForm";
import { href,navigate } from "../router";
export * from "../../features/deployments/DeploymentDetail";
const navigation = { href, navigate };
export function DeploymentDetail(props: Omit<ComponentProps<typeof feature.DeploymentDetail>, "navigation" | "healthContent">) {
  const healthForm = useMemo(() => monitoringForms({ workspace: props.workspace, permissions: props.permissions, ...navigation })["application-health"]!, [props.workspace, props.permissions]);
  const health = props.deploymentId && props.permissions.has("alerts.read") ? <ResourceForm config={healthForm} workspace={props.workspace} permissions={props.permissions} showHeading={false} showViewNavigation={false} /> : undefined;
  return <feature.DeploymentDetail {...props} navigation={navigation} healthContent={health} />;
}
