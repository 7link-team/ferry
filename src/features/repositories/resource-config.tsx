import { deleteResource } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, form, list } from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import {
  repositoryEventForm,
  repositoryEventList,
} from "../repositories/event-config";

export interface CodeConnection {
  id: string;
  name: string;
  provider: string;
  credential_id: string;
  account: string;
  state: string;
  authorization: string;
  description: string;
  base_url: string;
  api_url: string;
  organization: string;
  url?: string;
  version: number;
}

export function repositoriesLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "repository-events": repositoryEventList(workspace),
    "code-connections": list<CodeConnection>({
      resource: "code-connections",
      permissionResource: "repositories",
      rowKey: (row) => row.id,
      filters: [
        {
          key: "q",
          label: "名称",
          labelKey: "common-domain:common.name",
          type: "text",
        },
      ],
      columns: [
        {
          key: "name",
          label: "连接名称",
          labelKey: "resource-domain:repository.connectionName",
        },
        {
          key: "provider",
          label: "平台",
          labelKey: "resource-domain:repository.provider",
        },
        {
          key: "account",
          label: "账号",
          labelKey: "resource-domain:credential.account",
        },
        {
          key: "state",
          label: "连接状态",
          labelKey: "resource-domain:repository.connectionState",
          render: (row) =>
            ({
              connected: commonText("resource-domain:runner.connected"),
              expired: commonText("resource-domain:repository.authExpired"),
              refreshable: commonText(
                "resource-domain:repository.authRefreshable",
              ),
              failed: commonText("resource-domain:repository.connectionFailed"),
            })[row.state] ?? row.state,
        },
        {
          key: "description",
          label: "备注",
          labelKey: "resource-domain:resource.notes",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          permission: "repositories.read",
          to: (row) => ({ id: row.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "repositories.update",
          to: (row) => ({ id: row.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "repositories.delete",
          destructive: true,
          action: (row) =>
            deleteResource(workspace, "code-connections", row.id),
        },
      ],
      toolbar: {
        create: {
          label: "创建连接",
          labelKey: "resource-domain:repository.createConnection",
        },
      },
      empty: "尚无仓库连接。创建连接后，在流水线的拉取代码节点选择仓库和分支。",
      emptyKey: "resource-domain:repository.noConnections",
    }),
  };
}

export function repositoriesForms({}: ResourceConfigContext): Record<
  string,
  FormConfig<AnyRow>
> {
  return {
    "repository-events": repositoryEventForm(),
    "code-connections": form<CodeConnection>({
      resource: "code-connections",
      permissionResource: "repositories",
      heading: (row) => row.name,
      blank: {
        name: "",
        provider: "",
        credential_id: "0",
        base_url: "",
        api_url: "",
        organization: "",
        description: "",
      },
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "连接名称",
              labelKey: "resource-domain:repository.connectionName",
              type: "text",
              required: true,
            },
            {
              key: "source",
              label: "连接平台",
              labelKey: "resource-domain:repository.connectProvider",
              type: "repository-source",
              span: 2,
            },
            {
              key: "description",
              label: "备注",
              labelKey: "resource-domain:resource.notes",
              type: "textarea",
              span: 2,
            },
          ],
        },
      ],
    }),
  };
}
