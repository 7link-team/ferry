import { Plus, Trash2 } from "lucide-react";
import { useEffect } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText, hostRepositoryText } from "../../shared/i18n/i18n";
import { localeNames, supportedLocales } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";

export type NotificationChannel = { name: string; webhook: string; language: string; enabled: boolean };

// Match the official endpoint checks in notifications/webhook.rs; never infer
// a provider from a substring, query parameter or look-alike domain.
export function notificationProvider(value: string): string {
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || url.port) return "Webhook";
    const path = url.pathname, segments = path.replace(/^\//, "").split("/");
    switch (url.hostname) {
      case "qyapi.weixin.qq.com": if (path === "/cgi-bin/webhook/send") return "WeCom"; break;
      case "oapi.dingtalk.com": if (path === "/robot/send") return "DingTalk"; break;
      case "open.feishu.cn": if (path.startsWith("/open-apis/bot/v2/hook/")) return "Feishu"; break;
      case "open.larksuite.com": if (path.startsWith("/open-apis/bot/v2/hook/")) return "Lark"; break;
      case "api.telegram.org": if (segments.length === 2 && segments[0]!.startsWith("bot") && segments[0]!.length > 3 && segments[1] === "sendMessage") return "Telegram"; break;
      case "hooks.slack.com": if (path.startsWith("/services/")) return "Slack"; break;
      case "discord.com": case "discordapp.com": if (path.startsWith("/api/webhooks/")) return "Discord"; break;
      case "chat.googleapis.com": if (segments.length === 4 && segments[0] === "v1" && segments[1] === "spaces" && segments[3] === "messages") return "Google Chat"; break;
    }
  } catch { /* An incomplete URL is validated by the form. */ }
  return "Webhook";
}

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
  const rename = (webhook: string, index: number) => {
    const provider = notificationProvider(webhook);
    const used = new Set(channels.filter((_, at) => at !== index).map(channel => channel.name.trim()));
    let name = provider, number = 2;
    while (used.has(name)) name = `${provider} ${number++}`;
    patch(index, { webhook, name });
  };
  const add = () => {
    let number = 1;
    while (names.includes(`Webhook ${number}`)) number++;
    onChange([...channels, { name: `Webhook ${number}`, webhook: "", language: "en-US", enabled: true }]);
  };
  return <section className="min-w-0 space-y-3">
    <div className="flex items-center justify-between gap-3">
      <h4 className="font-medium">{hostRepositoryText("repository-monitor.channels")}</h4>
      {!readOnly && <Button type="button" variant="outline" size="sm" disabled={channels.length >= 20} onClick={add}><Plus className="size-4" />{commonText("common.add")}</Button>}
    </div>
    {channels.map((channel, index) => <div key={index} className="min-w-0 space-y-3 rounded-md border bg-muted/20 p-3" data-notification-channel>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="min-w-0 break-words text-sm font-medium">{hostRepositoryText(`notification.provider.${notificationProvider(channel.webhook)}`)} · {index + 1}</span>
        <div className="flex items-center gap-3"><label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={channel.enabled} disabled={readOnly} onChange={event => patch(index, { enabled: event.target.checked })} />{hostRepositoryText("repository-monitor.061")}</label>{!readOnly && <Button type="button" variant="ghost" size="icon" aria-label={`${commonText("common.delete")} ${index + 1}`} onClick={() => onChange(channels.filter((_, at) => at !== index))}><Trash2 className="size-4" /></Button>}</div>
      </div>
      <AutoField label={hostRepositoryText("notification.webhookUrl")}><Input type="url" className="min-w-0" aria-label={`${hostRepositoryText("notification.webhookUrl")} ${index + 1}`} value={channel.webhook} readOnly={readOnly} maxLength={4096} onChange={event => rename(event.target.value, index)} /></AutoField>
      <AutoField label={hostRepositoryText("repository-monitor.notificationLanguage")}><select aria-label={`${hostRepositoryText("repository-monitor.notificationLanguage")} ${index + 1}`} className="h-9 w-full min-w-0 max-w-64 rounded-md border bg-background px-3 text-sm" disabled={readOnly} value={channel.language} onChange={event => patch(index, { language: event.target.value })}>{supportedLocales.map(locale => <option key={locale} value={locale}>{localeNames[locale]}</option>)}</select></AutoField>
    </div>)}
  </section>;
}
function validUrl(value: string) {
  try { const url = new URL(value); return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password && !url.hash; } catch { return false; }
}
