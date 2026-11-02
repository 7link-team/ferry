import type { ComponentProps } from "react";
import * as feature from "../../features/hosts/HostWorkspace";
import { href,navigate } from "../router";
export * from "../../features/hosts/HostWorkspace";
const navigation = { href, navigate };
export function HostWorkspace(props: Omit<ComponentProps<typeof feature.HostWorkspace>, "navigation">) {
  return <feature.HostWorkspace {...props} navigation={navigation} />;
}
