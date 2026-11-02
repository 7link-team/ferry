import type { ComponentProps } from "react";
import * as feature from "../../features/hosts/RunnerConnection";
import { href,navigate } from "../router";
export * from "../../features/hosts/RunnerConnection";
const navigation = { href, navigate };
export function RunnerConnection(props: Omit<ComponentProps<typeof feature.RunnerConnection>, "navigation">) {
  return <feature.RunnerConnection {...props} navigation={navigation} />;
}
