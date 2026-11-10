import { AutoField } from "../../shared/forms/auto-form";
import { commonText as t } from "../../shared/i18n/i18n";

export const isSms = (provider: unknown) => provider === "Aliyun SMS" || provider === "Tencent SMS";
export function SmsParameters({ names, values, readOnly, onChange }: {
  names: string[]; values: Record<string, string>; readOnly: boolean; onChange: (values: Record<string, string>) => void;
}) {
  return <div className="min-w-0 space-y-3">
    <p className="text-sm text-muted-foreground">{t("notifications:smsSendHelp")}</p>
    <div className="grid min-w-0 gap-3 sm:grid-cols-2">{names.map(name => <AutoField key={name} label={name}>
      {readOnly ? <span className="break-words text-sm">{values[name] || "—"}</span> : <input aria-label={name} className="h-11 w-full rounded-md border bg-background px-3 text-sm" maxLength={500} required value={values[name] ?? ""} onChange={event => onChange({ ...values, [name]: event.target.value })} />}
    </AutoField>)}</div>
  </div>;
}
