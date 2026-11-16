import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../i18n/i18n";
import { InlineAlert, RetryButton } from "./inline-alert";

let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("shows the message with a recovery action, technical detail on demand, and a close button", async () => {
  const retry = vi.fn(), dismiss = vi.fn();
  await act(async () => root.render(
    <InlineAlert action={<RetryButton onClick={retry} />} detail="Map keys must be unique at line 2" onDismiss={dismiss}>加载失败</InlineAlert>,
  ));
  const alert = host.querySelector('[role="alert"]')!;
  expect(alert.textContent).toContain("加载失败");
  // Technical text sits behind a disclosure, not in the sentence.
  expect(alert.querySelector("details")!.open).toBe(false);
  expect(alert.querySelector("summary")!.textContent).toBe("详细信息");
  expect(alert.querySelector("pre")!.textContent).toBe("Map keys must be unique at line 2");
  const buttons = [...alert.querySelectorAll("button")];
  await act(async () => buttons.find(b => b.textContent === "重试")!.click());
  expect(retry).toHaveBeenCalledOnce();
  await act(async () => buttons.find(b => b.getAttribute("aria-label") === "关闭")!.click());
  expect(dismiss).toHaveBeenCalledOnce();
});

it("renders just the message when there is nothing else to offer", async () => {
  await act(async () => root.render(<InlineAlert>只有消息</InlineAlert>));
  const alert = host.querySelector('[role="alert"]')!;
  expect(alert.textContent).toBe("只有消息");
  expect(alert.querySelectorAll("button, details")).toHaveLength(0);
});
