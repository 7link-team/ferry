import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { NotificationChannels, notificationProvider } from "./NotificationChannels";

it.each([
  ["https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=example", "WeCom"],
  ["https://oapi.dingtalk.com/robot/send?access_token=example", "DingTalk"],
  ["https://open.feishu.cn/open-apis/bot/v2/hook/example", "Feishu"],
  ["https://open.larksuite.com/open-apis/bot/v2/hook/example", "Lark"],
  ["https://api.telegram.org/botexample/sendMessage", "Telegram"],
  ["https://hooks.slack.com/services/example", "Slack"],
  ["https://discord.com/api/webhooks/example", "Discord"],
  ["https://chat.googleapis.com/v1/spaces/example/messages", "Google Chat"],
  ["https://qyapi.weixin.qq.com.example.invalid/cgi-bin/webhook/send", "Webhook"],
  ["http://qyapi.weixin.qq.com/cgi-bin/webhook/send", "Webhook"],
  ["https://qyapi.weixin.qq.com:8443/cgi-bin/webhook/send", "Webhook"],
  ["https://qyapi.weixin.qq.com/other", "Webhook"],
  ["not a URL", "Webhook"],
])("recognises only supported official endpoints: %s", (url, expected) => {
  expect(notificationProvider(url)).toBe(expected);
});

it("shows saved webhook addresses, independent language selectors and channel controls without JSON", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"), root = createRoot(host);
  const change = vi.fn(), validity = vi.fn();
  const channels = [
    { name: "Operations", webhook: "https://notify.example.invalid/ops", language: "zh-CN", enabled: true },
    { name: "Development", webhook: "https://notify.example.invalid/dev", language: "ja-JP", enabled: false },
  ];
  try {
    await act(async () => root.render(<NotificationChannels channels={channels} readOnly={false} onChange={change} onValidityChange={validity} />));
    expect([...host.querySelectorAll<HTMLInputElement>('input[type="url"]')].map(input => input.value)).toEqual(channels.map(channel => channel.webhook));
    expect([...host.querySelectorAll('select')].map(select => select.value)).toEqual(["zh-CN", "ja-JP"]);
    expect([...host.querySelectorAll('select')].every(select => select.options.length === 6)).toBe(true);
    expect(host.querySelector('textarea')).toBeNull();
    expect(validity).toHaveBeenLastCalledWith(false);
    const remove = host.querySelectorAll<HTMLButtonElement>('button[aria-label]')[1]!;
    await act(async () => remove.click());
    expect(change).toHaveBeenLastCalledWith([channels[0]]);
    await act(async () => root.render(<NotificationChannels channels={[channels[0]!, { ...channels[1]!, name: channels[0]!.name }]} readOnly={false} onChange={change} onValidityChange={validity} />));
    expect(validity).toHaveBeenLastCalledWith(true);
    await act(async () => root.render(<NotificationChannels channels={[{ ...channels[0]!, webhook: "file:///private" }]} readOnly={false} onChange={change} onValidityChange={validity} />));
    expect(validity).toHaveBeenLastCalledWith(true);
  } finally { await act(async () => root.unmount()); }
});
