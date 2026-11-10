import { useState } from "react";
import { NotificationParameters } from "./NotificationParameters";
import { commonText as t } from "../../shared/i18n/i18n";

/** Translate the flat Actions input contract into the shared message form. */
export function ActionNotification({ workspace, inputs, readOnly, onChange }: {
  workspace: string; inputs: Record<string, unknown>; readOnly: boolean;
  onChange: (inputs: Record<string, string>) => void;
}) {
  const [blocked, setBlocked] = useState(false);
  const value = {
    channel_id: inputs["channel-id"] ?? "", template_id: inputs["template-id"] ?? "",
    subject: inputs.subject ?? "", body: inputs.body ?? "", format: inputs.format ?? "text",
    to: String(inputs.to ?? "").split(/[\n,;]/).map(v => v.trim()).filter(Boolean),
    variables: Object.fromEntries(Object.entries(inputs).filter(([key]) => key.startsWith("variable-")).map(([key, value]) => [key.slice(9), String(value)])),
  };
  return <div className="space-y-2"><p className="text-sm text-muted-foreground">{t("notifications:nativeOnly")}</p><NotificationParameters workspace={workspace} value={value} readOnly={readOnly} invalid={setBlocked} onChange={next => {
    onChange({ "channel-id": String(next.channel_id ?? ""), ...(next.template_id ? { "template-id": String(next.template_id) } : {}),
      subject: String(next.subject ?? ""), body: String(next.body ?? ""), format: String(next.format ?? "text"),
      to: ((next.to ?? []) as string[]).join("\n"),
      ...Object.fromEntries(Object.entries((next.variables ?? {}) as Record<string, string>).map(([key, value]) => [`variable-${key}`, value])),
    });
  }} />{blocked && !readOnly && <p role="status" className="text-sm text-muted-foreground">{t("notifications:messageRequired")}</p>}</div>;
}
