import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { i18n } from "../shared/i18n/i18n";
import { AppHeader } from "./AppHeader";

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  delete document.documentElement.dataset.nativeClient;
  await i18n.changeLanguage("en-US");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  delete document.documentElement.dataset.nativeClient;
});

it("keeps ordinary browser chrome free of native connection navigation", async () => {
  await act(async () => root.render(<AppHeader actions={<button>Account</button>} />));
  expect(host.querySelector('[href="app-settings://connection"]')).toBeNull();
});

it.each([
  ["en-US", "Server connection"],
  ["zh-CN", "服务器连接"],
  ["ms-MY", "Sambungan pelayan"],
])("offers a localized nonprivileged settings link in the %s mobile shell", async (locale, label) => {
  await i18n.changeLanguage(locale);
  document.documentElement.dataset.nativeClient = "mobile";
  await act(async () => root.render(<AppHeader actions={<button>Account</button>} />));
  const link = host.querySelector<HTMLAnchorElement>('[href="app-settings://connection"]');
  expect(link).not.toBeNull();
  expect(link!.getAttribute("aria-label")).toBe(label);
  expect(link!.hasAttribute("data-client-connection-entry")).toBe(true);
  expect(link!.closest("header")).not.toBeNull();
});

it("recognizes late Android initialization without reopening the application", async () => {
  await act(async () => root.render(<AppHeader actions={<button>Account</button>} />));
  expect(host.querySelector('[data-client-connection-entry]')).toBeNull();
  await act(async () => {
    document.documentElement.dataset.nativeClient = "mobile";
    window.dispatchEvent(new Event("client-shell:ready"));
  });
  expect(host.querySelector('[href="app-settings://connection"]')).not.toBeNull();
});
