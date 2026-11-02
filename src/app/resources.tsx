import { activityLists } from "../features/activity/resource-config";
import { alertsForms, alertsLists } from "../features/alerts/resource-config";
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
import { navigate } from "./router";
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
): Record<string, ListConfig<AnyRow>> {
  const context = { workspace, permissions, onChanged, hostStates, navigate };
  const configs = {
    ...marketplaceLists(context),
    ...alertsLists(context),
    ...repositoriesLists(context),
    ...schedulingLists(context),
    ...maintenanceLists(context),
    ...templatesLists(context),
    ...workspacesLists(context),
    ...deploymentsLists(context),
    ...codeInsightsLists(context),
    ...pipelinesLists(context),
    ...hostsLists(context),
    ...tagsLists(context),
    ...variablesLists(context),
    ...credentialsLists(context),
    ...activityLists(context),
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
  const context = { workspace, permissions, hostStates: STATES, navigate };
  return {
    ...marketplaceForms(context),
    ...alertsForms(context),
    ...repositoriesForms(context),
    ...schedulingForms(context),
    ...maintenanceForms(context),
    ...templatesForms(context),
    ...workspacesForms(context),
    ...deploymentsForms(context),
    ...codeInsightsForms(context),
    ...pipelinesForms(context),
    ...hostsForms(context),
    ...variablesForms(context),
    ...credentialsForms(context),
    ...tagsForms(context),
  };
}
