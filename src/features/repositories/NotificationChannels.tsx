import { Plus, Trash2 } from "lucide-react";
import { useEffect } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText, hostRepositoryText } from "../../shared/i18n/i18n";
import { localeNames, supportedLocales } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";

export type NotificationChannel = { name: string; webhook: string; language: string; enabled: boolean };

export function NotificationChannels({ channels, readOnly, onChange, onValidityChange }: {
  channels: NotificationChannel[]; readOnly: boolean;
  onChange: (channels: NotificationChannel[]) => void;
  onValidityChange: (blocked: boolean) => void;
}) {
  const names = channels.map(channel => channel.name.trim());
  const blocked = channels.some(channel => !channel.name.trim() || !validUrl(channel.webhook))
    || new Set(names).size !== names.length || channels.length > 20;
  useEffect(() => { onValidityChange(!readOnly && blocked); return () => onValidityChange(false); }, [blocked, readOnly, onValidityChange]);
  const patch = (index: number, value: Partial<NotificationChannel>) => onChange(channels.map((channel, at) => at === index ? { ...channel, ...value } : channel));
  const add = () => {
    let number = 1;
    while (names.includes(`Webhook ${number}`)) number++;
    onChange([...channels, { name: `Webhook ${number}`, webhook: "", language: "en-US", enabled: true }]);
  };
  return <section className="space-y-3">
    <div className="flex items-center justify-between gap-3">
      <h4 className="font-medium">{hostRepositoryText("repository-monitor.channels")}</h4>
      {!readOnly && <Button type="button" variant="outline" size="sm" disabled={channels.length >= 20} onClick={add}><Plus className="size-4" />{commonText("common.add")}</Button>}
    </div>
    {channels.map((channel, index) => <div key={index} className="grid items-end gap-3 rounded-md border bg-muted/20 p-3 sm:grid-cols-[minmax(8rem,1fr)_minmax(12rem,2fr)_minmax(8rem,1fr)_auto]">
      <AutoField label={commonText("common.name")}><Input aria-label={`${commonText("common.name")} ${index + 1}`} value={channel.name} readOnly={readOnly} maxLength={120} onChange={event => patch(index, { name: event.target.value })} /></AutoField>
      <AutoField label="Webhook URL"><Input type="url" aria-label={`Webhook URL ${index + 1}`} value={channel.webhook} readOnly={readOnly} maxLength={4096} onChange={event => patch(index, { webhook: event.target.value })} /></AutoField>
      <AutoField label={hostRepositoryText("repository-monitor.notificationLanguage")}><select aria-label={`${hostRepositoryText("repository-monitor.notificationLanguage")} ${index + 1}`} className="h-9 w-full rounded-md border bg-background px-3 text-sm" disabled={readOnly} value={channel.language} onChange={event => patch(index, { language: event.target.value })}>{supportedLocales.map(locale => <option key={locale} value={locale}>{localeNames[locale]}</option>)}</select></AutoField>
      <div className="flex h-9 items-center gap-3"><label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={channel.enabled} disabled={readOnly} onChange={event => patch(index, { enabled: event.target.checked })} />{hostRepositoryText("repository-monitor.061")}</label>{!readOnly && <Button type="button" variant="ghost" size="icon" aria-label={`${commonText("common.delete")} ${index + 1}`} onClick={() => onChange(channels.filter((_, at) => at !== index))}><Trash2 className="size-4" /></Button>}</div>
    </div>)}
  </section>;
}
function validUrl(value: string) {
  try { const url = new URL(value); return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password && !url.hash; } catch { return false; }
}
