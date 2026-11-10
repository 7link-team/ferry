import { useEffect, useRef, useState } from "react";
import { getResource } from "../../shared/api/resource";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText as t } from "../../shared/i18n/i18n";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";

export function NotificationVariables({ workspace, templateId, values, readOnly, onChange, onValidityChange }: {
  workspace: string; templateId: string; values: Record<string, string>; readOnly: boolean;
  onChange: (values: Record<string, string>) => void; onValidityChange: (blocked: boolean) => void;
}) {
  const [data, setData] = useState<{ id: string; subject: string; body: string }>();
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    setData(undefined); setError("");
    if (templateId) void getResource<{ id: string; subject: string; body: string }>(workspace, "notification-templates", templateId)
      .then(row => { if (active) setData(row); }).catch(e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [workspace, templateId]);
  const names = data ? [...new Set([...`${data.subject}\n${data.body}`.matchAll(/\{\{\s*([^{}]+?)\s*\}\}/g)].map(m => m[1]!.trim()))] : [];
  const blocked = !!templateId && (!data || !!error || names.some(name => !values[name]));
  const validity=useRef(onValidityChange); validity.current=onValidityChange;
  useEffect(() => { validity.current(blocked); }, [blocked]);
  if (!templateId) return null;
  if (error) return <p role="alert" className="text-sm text-destructive">{error}</p>;
  if (!data) return <LoadingSkeleton variant="form" />;
  return <div className="grid min-w-0 gap-3 sm:grid-cols-2">{names.map(name => <AutoField key={name} label={name}>
    {readOnly ? <span className="break-words text-sm">{values[name] || "—"}</span> : <input className="h-10 w-full rounded-md border bg-background px-3 text-sm" maxLength={48000} value={values[name] ?? ""} onChange={e => onChange({ ...values, [name]: e.target.value })} />}
  </AutoField>)}{!names.length && <p className="text-sm text-muted-foreground">{t("notifications:noVariables")}</p>}</div>;
}
