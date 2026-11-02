import type { ComponentProps } from "react";
import * as feature from "../../features/code-insights/CodeInsightFields";
import { href,navigate } from "../router";
export * from "../../features/code-insights/CodeInsightFields";
const navigation = { href, navigate };
export function CodeInsightRepositories(props: Omit<ComponentProps<typeof feature.CodeInsightRepositories>, "navigation">) {
  return <feature.CodeInsightRepositories {...props} navigation={navigation} />;
}
export function CodeInsightAliases(props: Omit<ComponentProps<typeof feature.CodeInsightAliases>, "navigation">) {
  return <feature.CodeInsightAliases {...props} navigation={navigation} />;
}
