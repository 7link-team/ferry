import { useEffect, useRef, useState } from "react";
import { commonText as t } from "../../shared/i18n/i18n";
import { AutoFields } from "../../shared/forms/auto-form";
import type { FormConfig } from "../../shared/forms/auto-form/types";
import { namedOptions } from "../../shared/resources/config";
import type { Option } from "../../shared/resources/types";
import { getResource } from "../../shared/api/resource";
import { isSms, SmsParameters } from "./SmsParameters";
import { NotificationBody } from "./NotificationBody";
import { NotificationVariables } from "./NotificationVariables";

/** Shared message parameters for workflow nodes and build notifications. */
export function NotificationParameters({ workspace, value, readOnly, onChange, invalid }: {
  workspace: string; value: Record<string, unknown>; readOnly: boolean;
  onChange: (value: Record<string, unknown>) => void; invalid: (blocked: boolean) => void;
}) {
  const [channels, setChannels] = useState<Option[]>([]);
  const [templates, setTemplates] = useState<Option[]>([]);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    setChannels([]); setTemplates([]); setError("");
    void Promise.all([namedOptions(workspace, "notification-channels")(), namedOptions(workspace, "notification-templates")()])
      .then(([a,b]) => { if (active) { setChannels(a); setTemplates(b); } }).catch(e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [workspace]);
  const [channel, setChannel] = useState<{provider:string;sms_parameter_names:string[]}>();
  const [channelError,setChannelError] = useState("");
  useEffect(() => {
    let active=true; setChannel(undefined); setChannelError("");
    if(value.channel_id) void getResource<{provider:string;sms_parameter_names:string[]}>(workspace,"notification-channels",String(value.channel_id)).then(row => {if(active)setChannel(row);}).catch(e => {if(active)setChannelError(String(e));});
    return () => {active=false;};
  },[workspace,value.channel_id]);
  const sms=isSms(channel?.provider);
  const [variablesBlocked, setVariablesBlocked] = useState(false);
  const blocked = !!error || !!channelError || !channel || !value.channel_id || (sms ? (channel.sms_parameter_names ?? []).some(name => !((value.variables ?? {}) as Record<string,string>)[name]) : (value.template_id ? variablesBlocked : !String(value.body ?? "").trim()));
  const validity=useRef(invalid); validity.current=invalid;
  useEffect(() => { validity.current(!!blocked); }, [blocked]);
  const fields: FormConfig = {
    channel_id: { name: t("notifications:notification-channels"), type: "select", required: true, width: "full", config: { options: channels } },
    template_id: { name: t("notifications:template_id"), type: "select", width: "full", show: () => !sms, config: { options: templates } },
    subject: { name: t("notifications:subject"), type: "text", col: 2, show: () => !sms && !value.template_id, config: { maxLength: 256 } },
    format: { name: t("notifications:format"), type: "select", show: () => !sms && !value.template_id, config: { options: [{ value: "text", label: t("notifications:text") }, { value: "html", label: "HTML" }] } },
    body: { name: t("notifications:body"), type: "custom", col: 2, show: () => !sms && !value.template_id, render: props => <NotificationBody value={String(props.value ?? "")} format={String(value.format ?? "text")} readOnly={props.readonly} onChange={props.onChange} /> },
    to: { name: t("notifications:to_text"), type: "custom", col: 2, help: t(sms ? "notifications:smsRecipientsHelp" : "notifications:recipientsHelp"), render: props => props.readonly ? <span>{(props.value as string[] ?? []).join(", ") || "—"}</span> : <textarea aria-label={t("notifications:to_text")} className="min-h-24 w-full rounded-md border bg-background p-3 text-sm" value={(props.value as string[] ?? []).join("\n")} onChange={e => props.onChange(e.target.value.split("\n"))} /> },
    variables: { name: t("notifications:variables"), type: "custom", col: 2, show: () => sms || !!value.template_id, render: props => sms ? <SmsParameters names={channel?.sms_parameter_names ?? []} values={(props.value ?? {}) as Record<string,string>} readOnly={props.readonly} onChange={props.onChange} /> : <NotificationVariables workspace={workspace} templateId={String(value.template_id ?? "")} values={(props.value ?? {}) as Record<string,string>} readOnly={props.readonly} onChange={props.onChange} onValidityChange={setVariablesBlocked} /> },
  };
  return <div className="min-w-0 space-y-3">{(error || channelError) && <p role="alert" className="text-sm text-destructive">{error || channelError}</p>}<AutoFields fields={fields} values={value} readonly={readOnly} onChange={patch => {const next={...value,...patch}; if(next.channel_id !== value.channel_id) {next.template_id="";next.format="text";next.variables={};} else if(sms) {next.template_id="";next.format="text";} onChange(next);}} /></div>;
}
