import type { ComponentProps } from "react";
import * as feature from "../../features/deployments/DeploymentDetail";
import { href,navigate } from "../router";
export * from "../../features/deployments/DeploymentDetail";
const navigation = { href, navigate };
export function DeploymentDetail(props: Omit<ComponentProps<typeof feature.DeploymentDetail>, "navigation">) {
  return <feature.DeploymentDetail {...props} navigation={navigation} />;
}
