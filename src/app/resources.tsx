import { monitoringForms, monitoringLists } from "../features/monitoring/resource-config";
import { notificationForms, notificationLists } from "../features/notifications/resource-config";
import { workflowsForms, workflowsLists } from "../features/workflows/resource-config";
import { activityLists } from "../features/activity/resource-config";
import { agentsForms, agentsLists } from "../features/agents/resource-config";
import { modelsForms, modelsLists } from "../features/models/resource-config";
import { conversationForms, conversationLists } from "../features/conversations/resource-config";
import { alertsForms, alertsLists } from "../features/alerts/resource-config";
import { holesForms, holesLists } from "../features/holes/resource-config";
import {
  codeInsightsForms,
  codeInsightsLists,
} from "../features/code-insights/resource-config";
import {
  credentialsForms,
  credentialsLists,
} from "../features/credentials/resource-config";
import {
  deploymentsForms,
  deploymentsLists,
} from "../features/deployments/resource-config";
import {
  hostsForms,
  hostsLists,
  STATES,
} from "../features/hosts/resource-config";
import {
  maintenanceForms,
  maintenanceLists,
} from "../features/maintenance/resource-config";
import {
  marketplaceForms,
  marketplaceLists,
} from "../features/marketplace/resource-config";
import {
  pipelinesForms,
  pipelinesLists,
} from "../features/pipelines/resource-config";
import {
  repositoriesForms,
  repositoriesLists,
} from "../features/repositories/resource-config";
import {
  schedulingForms,
  schedulingLists,
} from "../features/scheduling/resource-config";
import { tagsForms, tagsLists } from "../features/tags/resource-config";
import {
  templatesForms,
  templatesLists,
} from "../features/templates/resource-config";
import {
  variablesForms,
  variablesLists,
} from "../features/variables/resource-config";
import {
  workspacesForms,
  workspacesLists,
} from "../features/workspaces/resource-config";
import type { AnyRow } from "../shared/resources/config";
import type { FormConfig, ListConfig, Option } from "../shared/resources/types";
import { exportResources, importResources } from "./resources/ResourceTransfer";
import { href, navigate } from "./router";
import { ResourceTable } from "./resources/ResourceTable";
export type { Activity } from "../features/activity/resource-config";
export type { Credential } from "../features/credentials/types";
export type { Host, HostGroup } from "../features/hosts/resource-config";
export type { Tag } from "../features/tags/resource-config";
export type { Variable } from "../features/variables/resource-config";

export function listConfigs(
  workspace: string,
  permissions?: Set<string>,
  onChanged?: () => void,
  hostStates: Option[] = STATES,
  dictionaries: Record<string, Option[]> = {},
): Record<string, ListConfig<AnyRow>> {
  const context = { workspace, permissions, onChanged, hostStates, dictionaries, navigate, href };
  const configs = {
    ...marketplaceLists(context),
    ...alertsLists(context),
    ...monitoringLists(context),
    ...notificationLists(context),
    ...repositoriesLists(context),
    ...schedulingLists(context),
    ...maintenanceLists(context),
    ...templatesLists(context),
    ...workspacesLists(context),
    ...deploymentsLists(context),
    ...codeInsightsLists(context),
    ...pipelinesLists(context),
    ...workflowsLists(context),
    ...hostsLists(context),
    ...holesLists(context),
    ...tagsLists(context),
    ...variablesLists(context),
    ...credentialsLists(context),
    ...activityLists(context),
    ...agentsLists(context),
    ...modelsLists(context),
    ...conversationLists(context),
  };
  return Object.fromEntries(
    Object.entries(configs).map(([key, config]) => [
      key,
      {
        ...config,
        toolbar: {
          ...config.toolbar,
          import: !config.endpoint && importResources.has(config.resource),
          export: !config.endpoint && exportResources.has(config.resource),
        },
      },
    ]),
  );
}

export function formConfigs(
  workspace: string,
  permissions?: Set<string>,
): Record<string, FormConfig<AnyRow>> {
  const context = { workspace, permissions, hostStates: STATES, navigate, href,
    renderTable: (config: ListConfig<Record<string, unknown>>, refreshKey = 0) => <ResourceTable config={config} workspace={workspace} permissions={permissions ?? new Set()} refreshKey={refreshKey} />,
  };
  return {
    ...marketplaceForms(context),
    ...alertsForms(context),
    ...monitoringForms(context),
    ...notificationForms(context),
    ...repositoriesForms(context),
    ...schedulingForms(context),
    ...maintenanceForms(context),
    ...agentsForms(context),
    ...modelsForms(context),
    ...conversationForms(context),
    ...templatesForms(context),
    ...workspacesForms(context),
    ...deploymentsForms(context),
    ...codeInsightsForms(context),
    ...pipelinesForms(context),
    ...workflowsForms(context),
    ...hostsForms(context),
    ...holesForms(context),
    ...variablesForms(context),
    ...credentialsForms(context),
    ...tagsForms(context),
  };
}
