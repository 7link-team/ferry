import type { ComponentProps } from "react";
import * as feature from "../../features/repositories/RepositorySource";
import { href,navigate } from "../router";
export * from "../../features/repositories/RepositorySource";
const navigation = { href, navigate };
export function RepositorySource(props: Omit<ComponentProps<typeof feature.RepositorySource>, "navigation">) {
  return <feature.RepositorySource {...props} navigation={navigation} />;
}
