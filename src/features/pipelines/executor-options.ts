import { commonText } from "../../shared/i18n/i18n";
import { namedOptions } from "../../shared/resources/config";

export const pipelineExecutorOptions = (workspace: string, permissions?: Set<string>) => async () =>
  (await Promise.all(
    ["hosts", "host-groups"]
      .filter(resource => !permissions || permissions.has(`${resource}.read`))
      .map(async resource => (await namedOptions(workspace, resource)()).map(option => ({
        value: `${resource === "hosts" ? "host" : "host-group"}:${option.value}`,
        get label() {
          return `${commonText(resource === "hosts" ? "common-domain:nav.hosts" : "common-domain:nav.hostGroups")} · ${option.label}`;
        },
      }))),
  )).flat();
