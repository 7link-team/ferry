import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { NotificationChannels, notificationProvider, type NotificationChannel } from "./NotificationChannels";

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
    const languages = [...host.querySelectorAll('select')].filter(select => [...select.options].some(option => option.value === "en-US"));
    expect(languages.map(select => select.value)).toEqual(["zh-CN", "ja-JP"]);
    expect(languages.every(select => select.options.length === 6)).toBe(true);
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

it("supports provider fields, legacy Telegram URLs, secret redaction and platform mismatch validation", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const host = document.createElement("div"), root = createRoot(host), validity = vi.fn();
  let channels: NotificationChannel[] = [{ name: "Telegram", webhook: "https://api.telegram.org/bot123:fixture/sendMessage?chat_id=-100", language: "en-US", enabled: true }];
  let readOnly = false;
  const render = () => root.render(<NotificationChannels channels={channels} readOnly={readOnly} onChange={value => { channels = value; render(); }} onValidityChange={validity} />);
  const input = async (element: HTMLInputElement, value: string) => act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const select = async (provider: string) => act(async () => {
    const element = [...host.querySelectorAll('select')].find(select => [...select.options].some(option => option.value === "auto"))!;
    element.value = provider; element.dispatchEvent(new Event("change", { bubbles: true }));
  });
  try {
    await act(async () => render());
    await select("Telegram");
    expect(host.querySelector('input[type="url"]')).toBeNull();
    expect(host.querySelector<HTMLInputElement>('input[type="password"]')?.value).toBe("123:fixture");
    expect(channels[0]?.chat_id).toBe("-100");
    expect(validity).toHaveBeenLastCalledWith(false);
    await input(host.querySelector<HTMLInputElement>('input[type="password"]')!, "not-a-token");
    expect(validity).toHaveBeenLastCalledWith(true);
    await select("DingTalk");
    expect(channels[0]?.bot_token).toBe("");
    await input(host.querySelector<HTMLInputElement>('input[type="url"]')!, "https://hooks.slack.com/services/fixture");
    expect(validity).toHaveBeenLastCalledWith(true);
    await input(host.querySelector<HTMLInputElement>('input[type="url"]')!, "https://oapi.dingtalk.com/robot/send?access_token=fixture");
    expect(validity).toHaveBeenLastCalledWith(false);
    await input(host.querySelector<HTMLInputElement>('input[type="password"]')!, "secret-key");
    expect(channels[0]?.signing_secret).toBe("secret-key");
    readOnly = true; await act(async () => render());
    expect(host.querySelector<HTMLInputElement>('input[type="password"]')?.value).toBe("");
    expect(host.innerHTML).not.toContain("secret-key");
  } finally { await act(async () => root.unmount()); }
});
