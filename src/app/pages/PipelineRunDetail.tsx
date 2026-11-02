import type { ComponentProps } from "react";
import * as feature from "../../features/pipelines/PipelineRunDetail";
import { href,navigate } from "../router";
export * from "../../features/pipelines/PipelineRunDetail";
const navigation = { href, navigate };
export function PipelineRunDetail(props: Omit<ComponentProps<typeof feature.PipelineRunDetail>, "navigation">) {
  return <feature.PipelineRunDetail {...props} navigation={navigation} />;
}
