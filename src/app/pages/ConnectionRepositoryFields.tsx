import type { ComponentProps } from "react";
import * as feature from "../../features/repositories/ConnectionRepositoryFields";
import { href,navigate } from "../router";
export * from "../../features/repositories/ConnectionRepositoryFields";
const navigation = { href, navigate };
export function ConnectionRepositoryFields(props: Omit<ComponentProps<typeof feature.ConnectionRepositoryFields>, "navigation">) {
  return <feature.ConnectionRepositoryFields {...props} navigation={navigation} />;
}
