import catalog from "./workflow-integration-nodes.json";
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
  git?: { operation: string; provider?: string };
}
export const integrationManifests = [
  ...catalog.nodes,
] as unknown as IntegrationManifest[];
