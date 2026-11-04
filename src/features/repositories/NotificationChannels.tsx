import { Plus, Trash2 } from "lucide-react";
import { useEffect } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText, hostRepositoryText } from "../../shared/i18n/i18n";
import { localeNames, supportedLocales } from "../../shared/i18n/locale";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";

export type NotificationChannel = { name: string; webhook: string; language: string; enabled: boolean;
  provider?: string; signing_secret?: string; bot_token?: string; chat_id?: string; thread_id?: string; bearer_token?: string };
const providers = ["WeCom", "DingTalk", "Feishu", "Lark", "Telegram", "Slack", "Discord", "Google Chat", "Webhook"];
const instructions: Record<string, string> = {
  WeCom: "https://developer.work.weixin.qq.com/document/path/91770",
  DingTalk: "https://open.dingtalk.com/document/robots/custom-robot-access",
  Feishu: "https://open.feishu.cn/document/client-docs/bot-v3/add-custom-bot",
  Lark: "https://open.larksuite.com/document/client-docs/bot-v3/add-custom-bot",
  Telegram: "https://core.telegram.org/bots/api#sendmessage",
  Slack: "https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks/",
  Discord: "https://discord.com/developers/docs/resources/webhook",
  "Google Chat": "https://developers.google.com/workspace/chat/quickstart/webhooks",
};
function providerOf(channel: NotificationChannel) { return channel.provider && channel.provider !== "auto" ? channel.provider : notificationProvider(channel.webhook); }
function telegram(channel: NotificationChannel) {
  let token = "", chat = "", thread = "";
  if (notificationProvider(channel.webhook) === "Telegram") {
    const url = new URL(channel.webhook);
    token = url.pathname.split("/")[1]!.slice(3); chat = url.searchParams.get("chat_id") ?? ""; thread = url.searchParams.get("message_thread_id") ?? "";
  }
  return { bot_token: channel.bot_token ?? token, chat_id: channel.chat_id ?? chat, thread_id: channel.thread_id ?? thread };
}
function validChannel(channel: NotificationChannel) {
  const provider = providerOf(channel);
  if (channel.signing_secret && !["DingTalk", "Feishu", "Lark"].includes(provider)) return false;
  if (channel.bearer_token && provider !== "Webhook") return false;
  if ((channel.bot_token || channel.chat_id) && provider !== "Telegram") return false;
  if (channel.thread_id && !["Telegram", "Discord"].includes(provider)) return false;
  if (provider === "Telegram" && channel.provider === "Telegram") {
    const fields = telegram(channel);
    if (!/^\d+:[A-Za-z0-9_-]+$/.test(fields.bot_token) || !fields.chat_id.trim()) return false;
  } else if (!validUrl(channel.webhook) || (channel.provider && !["auto", "Webhook"].includes(channel.provider) && notificationProvider(channel.webhook) !== channel.provider)) return false;
  if (provider === "Telegram" && !telegram(channel).chat_id.trim()) return false;
  return !channel.thread_id || (/^[1-9]\d*$/.test(channel.thread_id) && BigInt(channel.thread_id) <= 18446744073709551615n);
}

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
  const blocked = channels.some(channel => !channel.name.trim() || !validChannel(channel))
    || new Set(names).size !== names.length || channels.length > 20;
  useEffect(() => { onValidityChange(!readOnly && blocked); return () => onValidityChange(false); }, [blocked, readOnly, onValidityChange]);
  const patch = (index: number, value: Partial<NotificationChannel>) => onChange(channels.map((channel, at) => at === index ? { ...channel, ...value } : channel));
  const rename = (webhook: string, index: number) => {
    const provider = channelProvider(index, webhook);
    const used = new Set(channels.filter((_, at) => at !== index).map(channel => channel.name.trim()));
    let name = provider, number = 2;
    while (used.has(name)) name = `${provider} ${number++}`;
    patch(index, { webhook, name, ...(providerOf(channels[index]!) !== provider ? { signing_secret: "", bot_token: "", chat_id: "", thread_id: "", bearer_token: "" } : {}) });
  };
  const channelProvider = (index: number, webhook: string) => {
    const selected = channels[index]?.provider;
    return selected && selected !== "auto" ? selected : notificationProvider(webhook);
  };
  const selectProvider = (index: number, provider: string) => {
    const channel = channels[index]!;
    const same = provider === providerOf(channel) || provider === "auto";
    const resolved = same && provider === "Telegram" ? telegram(channel) : {};
    patch(index, { provider, name: uniqueName(provider === "auto" ? notificationProvider(channel.webhook) : provider, index),
      ...(!same ? { webhook: "", signing_secret: "", bot_token: "", chat_id: "", thread_id: "", bearer_token: "" } : {}), ...resolved });
  };
  const uniqueName = (provider: string, index: number) => {
    const used = new Set(channels.filter((_, at) => at !== index).map(channel => channel.name.trim()));
    let name = provider, number = 2;
    while (used.has(name)) name = `${provider} ${number++}`;
    return name;
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
    {channels.map((channel, index) => {
      const provider = providerOf(channel), explicitTelegram = channel.provider === "Telegram";
      const textField = (key: "signing_secret" | "bot_token" | "chat_id" | "thread_id" | "bearer_token", secret = false, required = false) => {
        const label = hostRepositoryText(`notification.${key}`);
        const value = explicitTelegram && ["bot_token", "chat_id", "thread_id"].includes(key) ? telegram(channel)[key as "bot_token" | "chat_id" | "thread_id"] : channel[key] ?? "";
        return <AutoField label={label} required={required}><Input aria-label={`${label} ${index + 1}`} type={secret ? "password" : "text"} autoComplete="off" value={readOnly && secret ? "" : value} readOnly={readOnly} maxLength={4096} required={required} onChange={event => patch(index, { ...(explicitTelegram ? telegram(channel) : {}), [key]: event.target.value })} /></AutoField>;
      };
      return <div key={index} className="min-w-0 space-y-3 rounded-md border bg-muted/20 p-3" data-notification-channel>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="min-w-0 break-words text-sm font-medium">{hostRepositoryText(`notification.provider.${provider}`)} · {index + 1}</span>
        <div className="flex items-center gap-3"><label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={channel.enabled} disabled={readOnly} onChange={event => patch(index, { enabled: event.target.checked })} />{hostRepositoryText("repository-monitor.061")}</label>{!readOnly && <Button type="button" variant="ghost" size="icon" aria-label={`${commonText("common.delete")} ${index + 1}`} onClick={() => onChange(channels.filter((_, at) => at !== index))}><Trash2 className="size-4" /></Button>}</div>
      </div>
      <AutoField label={hostRepositoryText("notification.providerLabel")}><select aria-label={`${hostRepositoryText("notification.providerLabel")} ${index + 1}`} className="h-9 w-full min-w-0 max-w-64 rounded-md border bg-background px-3 text-sm" disabled={readOnly} value={channel.provider || "auto"} onChange={event => selectProvider(index, event.target.value)}><option value="auto">{hostRepositoryText("notification.auto")}</option>{providers.map(provider => <option key={provider} value={provider}>{hostRepositoryText(`notification.provider.${provider}`)}</option>)}</select></AutoField>
      {explicitTelegram ? <div className="grid min-w-0 gap-3 sm:grid-cols-2">{textField("bot_token", true, true)}{textField("chat_id", false, true)}</div> : <AutoField label={hostRepositoryText("notification.webhookUrl")} required><Input type="url" className="min-w-0" aria-label={`${hostRepositoryText("notification.webhookUrl")} ${index + 1}`} value={channel.webhook} readOnly={readOnly} maxLength={4096} onChange={event => rename(event.target.value, index)} /></AutoField>}
      {["DingTalk", "Feishu", "Lark"].includes(provider) && textField("signing_secret", true)}
      {["Telegram", "Discord"].includes(provider) && textField("thread_id")}
      {provider === "Webhook" && textField("bearer_token", true)}
      {instructions[provider] && <a className="inline-block text-sm text-primary underline underline-offset-4" href={instructions[provider]} target="_blank" rel="noreferrer">{hostRepositoryText("notification.instructions")}</a>}
      {!readOnly && !validChannel(channel) && <p className="text-sm text-destructive" role="status">{hostRepositoryText("notification.invalid")}</p>}
      <AutoField label={hostRepositoryText("repository-monitor.notificationLanguage")}><select aria-label={`${hostRepositoryText("repository-monitor.notificationLanguage")} ${index + 1}`} className="h-9 w-full min-w-0 max-w-64 rounded-md border bg-background px-3 text-sm" disabled={readOnly} value={channel.language} onChange={event => patch(index, { language: event.target.value })}>{supportedLocales.map(locale => <option key={locale} value={locale}>{localeNames[locale]}</option>)}</select></AutoField>
    </div>; })}
  </section>;
}
function validUrl(value: string) {
  try { const url = new URL(value); return ["http:", "https:"].includes(url.protocol) && !url.username && !url.password && !url.hash; } catch { return false; }
}
