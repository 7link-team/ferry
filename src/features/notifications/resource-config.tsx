import { AutoField } from "../../shared/forms/auto-form";
import { api } from "../../shared/api/api";
import { Send } from "lucide-react";
import { base, deleteResource } from "../../shared/api/resource";
import { commonText as t } from "../../shared/i18n/i18n";
import { supportedLocales } from "../../shared/i18n/locale";
import { cronDate, form, list, namedOptions, type AnyRow, type ResourceConfigContext } from "../../shared/resources/config";
import type { FormConfig, FormField, ListConfig, Option, RowAction } from "../../shared/resources/types";
import { isSms, SmsParameters } from "./SmsParameters";
import { NotificationBody } from "./NotificationBody";
import { NotificationVariables } from "./NotificationVariables";

const key = (name: string) => `notifications:${name}`;
const field = (name: string, type: FormField<AnyRow>["type"] = "text", more: Partial<FormField<AnyRow>> = {}): FormField<AnyRow> => ({ key: name, label: "", labelKey: key(name), type, ...more });
export const notificationProviders: Option[] = ["SMTP", "Aliyun SMS", "Tencent SMS", "WeCom", "DingTalk", "Feishu", "Lark", "Slack", "Telegram", "Discord", "Google Chat", "Webhook"].map(value => ({ value, label: value }));
const channelProviders = {
  email: notificationProviders.filter(p => p.value === "SMTP"),
  sms: notificationProviders.filter(p => isSms(p.value)),
  platforms: notificationProviders.filter(p => p.value !== "SMTP" && !isSms(p.value)),
};
const formats = [{ value: "text", label: "", labelKey: key("text") }, { value: "html", label: "HTML" }];
const states = ["sending", "succeeded", "failed", "unknown"].map(value => ({ value, label: "", labelKey: key(value) }));
const languageOptions = supportedLocales.map(value => ({ value, label: value }));
const notSmtp = (r: AnyRow) => r.provider !== "SMTP" && !isSms(r.provider);
const smtp = (r: AnyRow) => r.provider === "SMTP";
const bodyField = field("body", "custom", { span: 2, render: ({ draft, readOnly, onPatch }) => <NotificationBody value={String(draft.body ?? "")} format={String(draft.format ?? "text")} readOnly={readOnly} onChange={body => onPatch({ body })} /> });

function messageFields(workspace: string, row: AnyRow): FormField<AnyRow>[] {
  if (isSms(row.provider)) return [
    field("to_text", "textarea", { helpKey: key("smsRecipientsHelp"), span: 2 }),
    field("variables", "custom", { span: 2, render: ({draft,readOnly,onPatch}) => <SmsParameters names={row.sms_parameter_names ?? []} values={draft.variables ?? {}} readOnly={readOnly} onChange={variables => onPatch({variables})} /> }),
  ];
  return [
    field("template_id", "select", { options: namedOptions(workspace, "notification-templates"), resetOnChange: ["variables"], width: "full" }),
    field("to_text", "textarea", { helpKey: key("recipientsHelp"), span: 2 }),
    field("subject", "text", { when: r => !r.template_id, max: 256, span: 2 }),
    field("format", "select", { options: formats, when: r => !r.template_id }),
    { ...bodyField, when: r => !r.template_id },
    field("variables", "custom", { when: r => !!r.template_id, span: 2, render: ({ draft, readOnly, onPatch, onValidityChange }) => <NotificationVariables workspace={workspace} templateId={String(draft.template_id ?? "")} values={(draft.variables ?? {}) as Record<string, string>} readOnly={readOnly} onChange={variables => onPatch({ variables })} onValidityChange={onValidityChange} /> }),
  ];
}
const recipients = (value: unknown) => String(value ?? "").split(/[\n,;]/).map(v => v.trim()).filter(Boolean);

function sendAction(workspace: string, navigate: ResourceConfigContext["navigate"]): RowAction<AnyRow> {
  return {
    label: "发送", labelKey: key("send"), icon: <Send className="size-4" />, permission: "notifications.send", when: row => row.enabled,
    parameters: row => messageFields(workspace, row), dialogWidth: "wide", descriptionKey: key("sendHelp"),
    initialValues: () => ({ client_id: crypto.randomUUID(), subject: "", body: "", format: "text", variables: {} }),
    action: async (row, values = {}) => {
      const result = await api<{ id: string; state: string }>(`${base(workspace, "notification-channels")}/${encodeURIComponent(row.id)}/deliveries`, {
        method: "POST", body: JSON.stringify({ client_id: values.client_id, subject: values.subject ?? "", body: values.body ?? "", format: values.format ?? "text", template_id: values.template_id || null, to: recipients(values.to_text), variables: values.variables ?? {} }),
      });
      navigate({ group: "ops", item: "notification-deliveries", id: result.id, mode: "view" });
    },
  };
}

export function notificationLists({ workspace, navigate }: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  const actions = (resource: string): RowAction<AnyRow>[] => [
    { label: "查看", labelKey: "common-domain:common.view", to: r => ({ id: r.id, mode: "view" }) },
    { label: "编辑", labelKey: "common-domain:common.edit", permission: "notifications.update", to: r => ({ id: r.id, mode: "edit" }) },
    { label: "删除", labelKey: "common-domain:common.delete", permission: "notifications.delete", destructive: true, action: r => deleteResource(workspace, resource, r.id) },
  ];
  const configs: Record<string, ListConfig<AnyRow>> = {
    "notification-channels": list<AnyRow>({ resource: "notification-channels", permissionResource: "notifications", rowKey: r => r.id,
      filters: [{ key: "q", label: "", labelKey: key("name"), type: "text" }, { key: "provider", label: "", labelKey: key("provider"), type: "select", options: notificationProviders }],
      columns: [{ key: "name", label: "", labelKey: key("name"), sortable: true }, { key: "provider", label: "", labelKey: key("provider") }, { key: "enabled", label: "", labelKey: key("enabled"), render: r => t(r.enabled ? "common-domain:common.yes" : "common-domain:common.no") }, { key: "created_at", label: "", labelKey: key("created_at"), sortable: true, render: r => cronDate(r.created_at) }],
      actions: [sendAction(workspace, navigate), ...actions("notification-channels")], toolbar: { create: true },
    }),
    "notification-templates": list<AnyRow>({ resource: "notification-templates", permissionResource: "notifications", rowKey: r => r.id,
      filters: [{ key: "q", label: "", labelKey: key("name"), type: "text" }, { key: "language", label: "", labelKey: key("language"), type: "select", options: languageOptions }],
      columns: ["name", "language", "subject", "format"].map(name => ({ key: name, label: "", labelKey: key(name) })), actions: actions("notification-templates"), toolbar: { create: true },
    }),
    "notification-deliveries": list<AnyRow>({ resource: "notification-deliveries", permissionResource: "notifications", rowKey: r => r.id,
      filters: [{ key: "q", label: "", labelKey: key("subject"), type: "text" }, { key: "state", label: "", labelKey: key("state"), type: "select", options: states }],
      columns: [{ key: "subject", label: "", labelKey: key("subject") }, { key: "state", label: "", labelKey: key("state"), render: r => t(key(r.state)) }, { key: "source", label: "", labelKey: key("source"), render: r => t(key(`source.${r.source}`)) }, { key: "result", label: "", labelKey: key("result") }, { key: "created_at", label: "", labelKey: key("created_at"), sortable: true, render: r => cronDate(r.created_at) }],
      actions: [{ label: "查看", labelKey: "common-domain:common.view", to: r => ({ id: r.id, mode: "view" }) }], toolbar: { create: false },
    }),
  };
  const base = configs["notification-channels"]!;
  const scoped = (providers: Option[]): ListConfig<AnyRow> => ({...base,
    fixed:{provider:providers.map(p => p.value).join(",")},
    filters:(base.filters ?? []).filter(f => f.key !== "provider" || providers.length > 1).map(f => f.key === "provider" ? {...f,options:providers} : f),
  });
  configs["notification-channels"] = scoped(channelProviders.email);
  configs["notification-sms"] = scoped(channelProviders.sms);
  configs["notification-platforms"] = scoped(channelProviders.platforms);
  return configs;
}

export function notificationForms({ workspace, navigate, permissions }: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  const configs: Record<string, FormConfig<AnyRow>> = {
    "notification-channels": form<AnyRow>({ resource: "notification-channels", permissionResource: "notifications", heading: r => r.name, presentation: "sections",
      blank: { name: "", description: "", provider: "WeCom", enabled: true, language: "en-US", role_ids: [], security: "starttls", port: 587, recipients: [], sms_region: "ap-guangzhou", sms_parameter_names: [] },
      editValues: r => ({ ...r, webhook: "", password: "", signing_secret: "", bearer_token: "", bot_token: "", sms_secret_key: "" }),
      viewActions: [sendAction(workspace, navigate)],
      sections: [{ titleKey: key("basics"), fields: [field("name", "text", { required: true, max: 120 }), field("enabled", "switch"), field("provider", "select", { required: true, options: notificationProviders, resetOnChange: ["webhook", "signing_secret", "bot_token", "bearer_token", "chat_id", "thread_id"] }), field("description", "textarea", { span: 2, max: 2048 })] },
        { titleKey: key("connection"), fields: [
          field("webhook", "password", { when: r => notSmtp(r) && r.provider !== "Telegram", stored: "has_webhook", width: "full", span: 2 }),
          field("bot_token", "password", { when: r => r.provider === "Telegram", stored: "has_secret" }), field("chat_id", "text", { when: r => r.provider === "Telegram", required: true }), field("thread_id", "text", { when: r => r.provider === "Telegram" }),
          field("signing_secret", "password", { when: r => ["DingTalk", "Feishu", "Lark"].includes(r.provider), stored: "has_secret" }), field("bearer_token", "password", { when: r => r.provider === "Webhook", stored: "has_secret" }),
          field("host", "text", { when: smtp, required: true }), field("port", "number", { when: smtp, min: 1, max: 65535, required: true }), field("security", "select", { when: smtp, options: ["tls", "starttls", "none"].map(value => ({ value, label: value === "none" ? "" : value.toUpperCase(), ...(value === "none" ? { labelKey: key("noTls") } : {}) })) }), field("from", "text", { when: smtp, required: true }), field("username", "text", { when: smtp }), field("password", "password", { when: smtp, stored: "has_secret" }), field("recipients", "custom", { when: r => smtp(r) || isSms(r.provider), span: 2, helpKey: key("smsDefaultRecipientsHelp"), render: ({draft,readOnly,onPatch}) => <AutoField label={t(key("recipients"))} help={t(isSms(draft.provider) ? key("smsRecipientsHelp") : key("recipientsHelp"))}>{readOnly ? <div className="whitespace-pre-wrap text-sm">{(draft.recipients ?? []).join("\n") || "—"}</div> : <textarea aria-label={t(key("recipients"))} className="min-h-24 w-full rounded-md border bg-background p-3 text-sm" value={(draft.recipients ?? []).join("\n")} onChange={event => onPatch({recipients:event.target.value.split("\n")})} />}</AutoField> }),
        ] }, { titleKey: key("smsConfiguration"), fields: [
          field("sms_access_key", "text", { when: r => isSms(r.provider), required: true, max: 128 }),
          field("sms_secret_key", "password", { when: r => isSms(r.provider), stored: "has_sms_secret", max: 256 }),
          field("sms_region", "select", { when: r => r.provider === "Tencent SMS", required: true, options: ["ap-guangzhou", "ap-beijing", "ap-nanjing"].map(value => ({value,label:value})) }),
          field("sms_app_id", "text", { when: r => r.provider === "Tencent SMS", required: true, max: 32 }),
          field("sms_sign_name", "text", { when: r => isSms(r.provider), required: true, max: 50 }),
          field("sms_template_code", "text", { when: r => isSms(r.provider), required: true, max: 128 }),
          field("sms_parameter_names", "custom", { when: r => isSms(r.provider), span: 2, helpKey: key("smsParameterNamesHelp"), render: ({draft,readOnly,onPatch}) => <AutoField label={t(key("sms_parameter_names"))} help={t(key("smsParameterNamesHelp"))}>{readOnly ? <div className="whitespace-pre-wrap text-sm">{(draft.sms_parameter_names ?? []).join("\n") || "\u2014"}</div> : <textarea aria-label={t(key("sms_parameter_names"))} className="min-h-24 w-full rounded-md border bg-background p-3 text-sm" value={(draft.sms_parameter_names ?? []).join("\n")} onChange={event => onPatch({sms_parameter_names:event.target.value.split("\n")})} />}</AutoField> }),
        ] }, { titleKey: key("authorization"), fields: [field("role_ids", "multiselect", { options: permissions?.has("roles.read") ? namedOptions(workspace, "roles") : [], searchable: true, span: 2, helpKey: key("rolesHelp") })] }],
    }),
    "notification-templates": form<AnyRow>({ resource: "notification-templates", permissionResource: "notifications", heading: r => r.name, blank: { name: "", language: "en-US", subject: "", body: "", format: "html" },
      sections: [{ fields: [field("name", "text", { required: true, max: 120 }), field("language", "select", { options: languageOptions, required: true }), field("subject", "text", { span: 2, max: 256 }), field("format", "select", { options: formats, required: true }), { ...bodyField, helpKey: key("templateHelp") }] }],
    }),
    "notification-deliveries": form<AnyRow>({ resource: "notification-deliveries", permissionResource: "notifications", heading: r => r.subject || t(key("notification-deliveries")), blank: {}, readOnly: () => true,
      sections: [{ fields: [field("subject"), field("state", "select", { options: states }), field("source"), field("source_id"), field("actor_id"), field("agent_id"), field("result", "textarea", { span: 2 }), field("body", "textarea", { span: 2 })] }],
    }),
  };
  const base = configs["notification-channels"]!;
  const scoped = (providers: Option[]): FormConfig<AnyRow> => ({
    ...base,
    blank: { ...base.blank, provider: providers[0]!.value },
    sections: base.sections.map(section => ({ ...section, fields: section.fields.map(field => field.key === "provider"
      ? { ...field, options: providers, disabled: () => providers.length === 1 }
      : field) })),
  });
  configs["notification-channels"] = scoped(channelProviders.email);
  configs["notification-sms"] = scoped(channelProviders.sms);
  configs["notification-platforms"] = scoped(channelProviders.platforms);
  return configs;
}
