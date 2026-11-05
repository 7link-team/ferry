import { deleteResource, listResource } from "../../shared/api/resource";
import { commonText, workflowText } from "../../shared/i18n/i18n";
import { integrationManifests } from "../workflows/integrations";
import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, form, label, list } from "../../shared/resources/config";
import type {
  FormConfig,
  ListConfig,
  Option,
} from "../../shared/resources/types";
import { Badge } from "../../shared/ui/badge";
import { AutoField } from "../../shared/forms/auto-form";
import type { Credential } from "./types";

export const CREDENTIAL_KINDS: Option[] = [
  { value: "password", label: "密码", labelKey: "common-domain:auth.password" },
  {
    value: "ssh-key",
    label: "SSH 私钥",
    labelKey: "resource-domain:credential.sshKey",
  },
  {
    value: "token",
    label: "访问令牌",
    labelKey: "resource-domain:credential.token",
  },
];
export const credentialScopes = (): Option[] => (["host", "build", "deploy", "ai", "integration"] as const).map(value => ({value,label:workflowText(`credentialScope.${value}`)}));

export const sshCredentialOptions =
  (workspace: string) => async (): Promise<Option[]> => {
    const credentials: Credential[] = [];
    for (let page = 1; ; page++) {
      const result = await listResource<Credential>(workspace, "credentials", {
        page,
        page_size: 200,
        scope: "host",
      });
      credentials.push(...result.items);
      if (page * result.page_size >= result.total) break;
    }
    return credentials
      .filter((c) => c.kind === "password" || c.kind === "ssh-key")
      .map((c) => ({
        value: c.id,
        label: `${c.name}（${label(CREDENTIAL_KINDS, c.kind)}）`,
      }));
  };

export function credentialsLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    credentials: list({
      resource: "credentials",
      rowKey: (row: Credential) => row.id,
      filters: [
        {key:"scope",label:workflowText("credentialScope"),type:"select",options:credentialScopes()},
        {
          key: "q",
          label: "名称/账号",
          labelKey: "resource-domain:credential.search",
          type: "text",
          placeholder: "输入关键字",
          placeholderKey: "resource-domain:resource.keyword",
        },
        {
          key: "kind",
          label: "类型",
          labelKey: "resource-domain:resource.type",
          type: "select",
          options: CREDENTIAL_KINDS,
        },
      ],
      columns: [
        {key:"scope",label:workflowText("credentialScope"),render:(row:Credential)=>label(credentialScopes(),row.scope)},
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        {
          key: "kind",
          label: "类型",
          labelKey: "resource-domain:resource.type",
          width: 120,
          render: (r: Credential) => label(CREDENTIAL_KINDS, r.kind),
        },
        {
          key: "username",
          label: "账号",
          labelKey: "resource-domain:credential.account",
          width: 160,
        },
        {
          key: "has_secret",
          label: "内容",
          labelKey: "common-domain:common.content",
          width: 100,
          render: (r: Credential) =>
            r.has_secret ? (
              <Badge variant="secondary">
                {commonText("common-domain:common.set")}
              </Badge>
            ) : (
              <Badge variant="destructive">
                {commonText("common-domain:common.notSet")}
              </Badge>
            ),
        },
      ],
      actions: [
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "credentials.update",
          to: (r: Credential) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "credentials.delete",
          destructive: true,
          action: (r: Credential) =>
            deleteResource(workspace, "credentials", r.id),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "当前模块还没有凭据，添加后可在本模块中选择使用。",
      emptyKey: "resource-domain:credential.empty",
    }),
  };
}

export function credentialsForms({}: ResourceConfigContext): Record<
  string,
  FormConfig<AnyRow>
> {
  return {
    credentials: form({
      resource: "credentials",
      heading: (r: Credential) => r.name,
      blank: {
        name: "",
        scope: "build",
        kind: "password",
        username: "",
        description: "",
        secret: "",
      },
      sections: [
        {
          fields: [
            {key:"scope",label:workflowText("credentialScope"),type:"custom",immutable:true,render:({draft,readOnly,onPatch})=><AutoField label={workflowText("credentialScope")}><select aria-label={workflowText("credentialScope")} className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm" disabled={readOnly} value={String(draft.scope ?? "build")} onChange={event=>onPatch({scope:event.target.value,kind:["ai","integration"].includes(event.target.value)?"token":(draft.kind??"password"),username:""})}>{credentialScopes().map(option=><option key={option.value} value={option.value}>{option.label}</option>)}</select></AutoField>},
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
              required: true,
              placeholder: "输入便于识别的凭据名称",
              placeholderKey: "resource-domain:credential.namePlaceholder",
            },
            {
              key: "kind",
              label: "类型",
              labelKey: "resource-domain:resource.type",
              type: "select",
              required: true,
              options: CREDENTIAL_KINDS,
              disabled: (draft) => draft.scope === "integration",
              immutable: true,
              help: "创建后不可更改：改类型等于换一条凭据。",
              helpKey: "resource-domain:credential.typeImmutable",
            },
            {
              key: "username",
              label: "账号",
              labelKey: "resource-domain:credential.account",
              type: "text",
              placeholder: "填写目标服务的登录账号",
              placeholderKey: "resource-domain:credential.accountPlaceholder",
              when: (draft) => draft.scope !== "integration",
            },
            {
              key: "username",
              label: workflowText("integration.application"),
              type: "select",
              required: true,
              immutable: true,
              options: [...new Map(integrationManifests.filter(node => node.http).map(node => [node.ui.application, { value: node.ui.application, label: node.ui.application_name }])).values()],
              when: (draft) => draft.scope === "integration",
            },
            {
              key: "secret",
              label: "内容",
              labelKey: "common-domain:common.content",
              type: "secret",
              span: 2,
              help: "密码、私钥全文或令牌。保存后服务端加密存放，任何接口都不会再返回它；编辑时留空表示不修改。",
              helpKey: "resource-domain:credential.secretHelp",
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
