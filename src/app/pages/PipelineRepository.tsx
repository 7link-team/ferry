import type { ComponentProps } from "react";
import * as feature from "../../features/pipelines/PipelineRepository";
import { href,navigate } from "../router";
export * from "../../features/pipelines/PipelineRepository";
const navigation = { href, navigate };
export function PipelineRepository(props: Omit<ComponentProps<typeof feature.PipelineRepository>, "navigation">) {
  return <feature.PipelineRepository {...props} navigation={navigation} />;
}
export function MarketplaceRepository(props: Omit<ComponentProps<typeof feature.MarketplaceRepository>, "navigation">) {
  return <feature.MarketplaceRepository {...props} navigation={navigation} />;
}
