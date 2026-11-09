import catalog from "../../shared/workflow-contracts/workflow-integration-nodes.json";
import { workflowText as t } from "../../shared/i18n/i18n";
import { setExtensionManifests, type Manifest } from "./graph";
import { integrationManifests, type IntegrationManifest } from "../../shared/workflow-contracts/integration-catalog";
export { integrationManifests, type IntegrationManifest, type IntegrationField } from "../../shared/workflow-contracts/integration-catalog";

export function installIntegrationCatalog(nodes: IntegrationManifest[]) {
  if (!Array.isArray(nodes) || nodes.some(node => !node.type || !node.ui || !node.params_schema))
    throw new Error("Invalid workflow node catalog");
  const builtins = new Set(catalog.nodes.map(node => node.type));
  setExtensionManifests(nodes.filter(node => !builtins.has(node.type)) as unknown as Manifest[]);
  integrationManifests.splice(0, integrationManifests.length, ...nodes);
}
export const integrationManifest = (type: string) =>
  integrationManifests.find((node) => node.type === type);
export const integrationText = (key: string) =>
  key.startsWith("literal:") ? key.slice(8) : t(key as Parameters<typeof t>[0]);
export const integrationOperationTitle = (node: IntegrationManifest) =>
  `${integrationText(node.ui.resource_key)} · ${integrationText(node.ui.operation_key)}`;
export const integrationTitle = (node: IntegrationManifest) =>
  `${node.ui.application_name} · ${integrationOperationTitle(node)}`;
