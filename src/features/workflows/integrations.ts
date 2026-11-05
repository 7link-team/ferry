import catalog from "../../shared/workflow-contracts/workflow-integration-nodes.json";
import { workflowText as t } from "../../shared/i18n/i18n";
import { setExtensionManifests, type Manifest } from "./graph";

export interface IntegrationField {
  type: string;
  title_key: string;
  enum?: string[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  control?: string;
  options_source?: {
    type: string;
    items: string;
    label: string;
    value: string;
  };
}
export interface IntegrationManifest {
  type: string;
  type_version: number;
  ui: {
    application: string;
    application_name: string;
    resource_key: string;
    operation_key: string;
    credential_help_key: string;
    aliases: string[];
  };
  params_schema: {
    properties: Record<string, IntegrationField>;
    required: string[];
  };
  example_params: Record<string, unknown>;
  http?: { method: string; auth: { optional?: boolean } };
  git?: { operation: string };
}
export const integrationManifests =
  [...catalog.nodes] as unknown as IntegrationManifest[];
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
