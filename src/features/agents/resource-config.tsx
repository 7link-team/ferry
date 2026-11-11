import { Eye, Pencil } from "lucide-react";
import { AvatarEditor, IdentityAvatar, type AvatarValue } from "../../shared/ui/identity-avatar";
import { AgentPrompt } from "./AgentPrompt";
import { useEffect, useState } from "react";
import { errorText } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../../shared/ui/select";
import { commonText as t } from "../../shared/i18n/i18n";
import { listResource, named } from "../../shared/api/resource";
import {
  form,
  list,
  label,
  type AnyRow,
  type ResourceConfigContext,
} from "../../shared/resources/config";
import type {
  Option,
  FormConfig,
  ListConfig,
} from "../../shared/resources/types";

interface Agent {
  avatar?: AvatarValue | undefined;
  id: string;
  name: string;
  role: string;
  platform_role_id: string;
  platform_role_name?: string;
  description: string;
  owner_id: string;
  owner_name?: string;
  runtime: string;
  model_profile_id: string;
  model_profile_name?: string;
  effective_model?: string;
  model: string;
  host_id: string;
  host_name?: string;
  cwd: string;
  system_prompt: string;
  permission: string;
  visibility: string;
  state: string;
  version: number;
  revision_id: string;
}
const key = (value: string) => `agents:${value}`;
const text = (value: string) => t(key(value));
const choices = (values: string[]): Option[] =>
  values.map((value) => ({ value, label: value, labelKey: key(value) }));
const states = choices(["enabled", "disabled", "archived"]);
const visibility = choices(["owner", "workspace"]);
const editable = (row: Agent) => ({
  avatar: row.avatar,
  name: row.name,
  role: row.role,
  platform_role_id: row.platform_role_id ?? "0",
  owner_id: row.owner_id,
  model_profile_id: row.model_profile_id,
  model: row.model,
  system_prompt: row.system_prompt,
  visibility: row.visibility,
  state: row.state,
  version: row.version,
});
interface Profile {
  id: string;
  name: string;
  models: string[];
  default_model: string;
  state: string;
}
function AgentModel({
  workspace,
  draft,
  readOnly,
  onPatch,
  onValidityChange,
}: {
  workspace: string;
  draft: Partial<Agent>;
  readOnly: boolean;
  onPatch: (patch: Partial<Agent>) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    if (readOnly) {
      setLoading(false);
      return;
    }
    const controller = new AbortController();
    async function load() {
      try {
        const items: Profile[] = [];
        for (let page = 1; ; page++) {
          const r = await listResource<Profile>(
            workspace,
            "ai-models",
            { page, page_size: 200 },
            controller.signal,
          );
          items.push(...r.items);
          if (page * r.page_size >= r.total || !r.items.length) break;
        }
        if (!controller.signal.aborted) {
          setProfiles(items);
          setError("");
          setLoading(false);
        }
      } catch (e) {
        if (!controller.signal.aborted) {
          setError(errorText(e));
          setLoading(false);
        }
      }
    }
    void load();
    return () => controller.abort();
  }, [workspace, readOnly]);
  const selected = profiles.find((p) => p.id === draft.model_profile_id);
  useEffect(() => {
    onValidityChange(
      !readOnly && (loading || !selected || selected.state !== "enabled"),
    );
  }, [loading, selected, readOnly, onValidityChange]);
  const catalog = [
    ...new Set(
      [...(selected?.models ?? []), selected?.default_model].filter(
        (s): s is string => !!s,
      ),
    ),
  ];
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <AutoField label={text("modelProfile")} required>
        {readOnly ? (
          <p className="text-sm">
            {selected?.name ?? draft.model_profile_name ?? "—"}
          </p>
        ) : (
          <Select
            value={
              draft.model_profile_id && draft.model_profile_id !== "0"
                ? draft.model_profile_id
                : ""
            }
            disabled={loading}
            onValueChange={(value) =>
              onPatch({ model_profile_id: value, model: "" })
            }
          >
            <SelectTrigger className="w-full" aria-label={text("modelProfile")}>
              <SelectValue placeholder={text("selectModelProfile")} />
            </SelectTrigger>
            <SelectContent>
              {profiles
                .filter(
                  (p) =>
                    p.state === "enabled" || p.id === draft.model_profile_id,
                )
                .map((p) => (
                  <SelectItem
                    key={p.id}
                    value={p.id}
                    disabled={p.state !== "enabled"}
                  >
                    {p.name}
                  </SelectItem>
                ))}
            </SelectContent>
          </Select>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </AutoField>
      <AutoField label={text("model")}>
        {readOnly ? (
          <p className="text-sm">
            {draft.model ||
              draft.effective_model ||
              selected?.default_model ||
              text("nativeModel")}
          </p>
        ) : (
          <Select
            value={draft.model || "-profile-default"}
            disabled={!selected || loading}
            onValueChange={(value) =>
              onPatch({ model: value === "-profile-default" ? "" : value })
            }
          >
            <SelectTrigger className="w-full" aria-label={text("model")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="-profile-default">
                {selected?.default_model || text("nativeModel")}
              </SelectItem>
              {catalog.map((model) => (
                <SelectItem key={model} value={model}>
                  {model}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </AutoField>
    </div>
  );
}
export function agentsLists(
  _: ResourceConfigContext,
): Record<string, ListConfig<AnyRow>> {
  return {
    "ai-agents": list<Agent>({
      resource: "ai-agents",
      rowKey: (r) => r.id,
      filters: [
        { key: "q", label: "Search", labelKey: key("search"), type: "text" },
        {
          key: "state",
          label: "State",
          labelKey: key("state"),
          type: "select",
          options: states,
        },
      ],
      columns: [
        { key: "name", label: "Name", labelKey: key("name"), sortable: true, render: row => <span className="flex min-w-0 items-center gap-2"><IdentityAvatar id={row.id} name={row.name} kind="agent" value={row.avatar} className="size-8" /><span className="truncate">{row.name}</span></span> },
        { key: "role", label: "Role", labelKey: key("role") },
        {
          key: "model_profile_name",
          label: "Model profile",
          labelKey: key("modelProfile"),
          render: (r) => r.model_profile_name || "—",
        },
        {
          key: "model",
          label: "Model",
          labelKey: key("model"),
          render: (r) => r.model || r.effective_model || text("nativeModel"),
        },
        {
          key: "owner_name",
          label: "Owner",
          labelKey: key("human"),
          render: (r) => named(r.owner_name, r.owner_id),
        },
        {
          key: "visibility",
          label: "Scope",
          labelKey: key("visibility"),
          render: (r) => label(visibility, r.visibility),
        },
        {
          key: "state",
          label: "State",
          labelKey: key("state"),
          render: (r) => label(states, r.state),
        },
      ],
      actions: [
        {
          label: "View",
          icon: <Eye className="size-4" />,
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "Edit",
          icon: <Pencil className="size-4" />,
          labelKey: "common-domain:common.edit",
          permission: "ai-agents.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "",
      emptyKey: key("empty"),
    }),
  };
}
export function agentsForms({
  workspace,
  permissions: grants,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  const members = async (): Promise<Option[]> => {
    const items: Option[] = [];
    for (let page = 1; ; page++) {
      const result = await listResource<{
        user_id: string;
        name: string;
        state: string;
      }>(workspace, "members", { page, page_size: 200 });
      items.push(
        ...result.items
          .filter((r) => r.state === "active")
          .map((r) => ({ value: r.user_id, label: r.name })),
      );
      if (page * result.page_size >= result.total || !result.items.length)
        return items;
    }
  };
  const roles = async (): Promise<Option[]> => {
    const items: Option[] = [{ value: "0", label: text("noPlatformRole") }];
    for (let page=1;;page++) {
      const result=await listResource<{id:string;name:string}>(workspace,"roles",{page,page_size:200});
      items.push(...result.items.map(row=>({value:row.id,label:row.name})));
      if (!result.items.length || page*result.page_size>=result.total) return items;
    }
  };
  return {
    "ai-agents": form<Agent>({
      resource: "ai-agents",
      heading: (r) => r.name,
      blank: {
        name: "",
        role: "",
        platform_role_id: "0",
        model_profile_id: "",
        model: "",
        owner_id: "",
        system_prompt: "",
        visibility: "owner",
        state: "enabled",
      },
      editValues: editable,
      sections: [
        {
          title: "Identity",
          titleKey: key("identity"),
          fields: [
            {
              key: "name",
              label: "Name",
              labelKey: key("name"),
              type: "text",
              required: true,
            },
            { key: "avatar", label: "Avatar", labelKey: "identity:avatar", type: "custom", span: 2,
              render: ({ draft, readOnly, onPatch, onValidityChange }) => <AvatarEditor id={draft.id ?? "new"} name={draft.name ?? ""} kind="agent" value={draft.avatar} readOnly={readOnly} onBusyChange={onValidityChange} onChange={avatar => onPatch({ avatar })} /> },
            { key: "role", label: "Role", labelKey: key("role"), type: "text" },
            {
              key: "platform_role_id", label: "Platform role", labelKey: key("platformRole"), type: "select",
              options: grants?.has("roles.read") ? roles : [],
              disabled: () => !grants?.has("roles.update"),
              display: (row) => row.platform_role_id && row.platform_role_id !== "0" ? named(row.platform_role_name,row.platform_role_id) : text("noPlatformRole"),
              helpKey: key("platformRoleHelp"),
            },
            {
              key: "owner_id",
              label: "Human",
              labelKey: key("human"),
              type: "select",
              required: true,
              options: grants?.has("members.read") ? members : [],
              display: (r) => named(r.owner_name, r.owner_id),
            },
            {
              key: "state",
              label: "State",
              labelKey: key("state"),
              type: "select",
              options: states,
            },
          ],
        },
        {
          title: "Configuration",
          titleKey: key("configuration"),
          fields: [
            {
              key: "model_profile_id",
              label: "Model profile",
              type: "custom",
              span: 2,
              render: (props) => <AgentModel {...props} />,
            },
            {
              key: "visibility",
              label: "Scope",
              labelKey: key("visibility"),
              type: "select",
              options: visibility,
              helpKey: key("visibilityHelp"),
            },
            {
              key: "system_prompt",
              label: "Instructions",
              labelKey: key("prompt"),
              type: "custom",
              span: 2,
              render: (props) => <AgentPrompt {...props} />,
            },
          ],
        },
      ],
    }),
  };
}
