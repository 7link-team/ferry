import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { i18n } from "../shared/i18n/i18n";
import { StorageStatus } from "./Platform";

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

it("shows working storage as a plain status", async () => {
  await act(async () => root.render(<StorageStatus status={{ ok: true }} />));
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(host.querySelector('[role="status"]')!.textContent).toContain("可用");
});

it("shows unavailable storage as an alert with the server's reason and its impact", async () => {
  const reason = "S3 拒绝访问：请检查 Access Key、Secret Key 及其对该 Bucket 的读写权限。";
  await act(async () => root.render(<StorageStatus status={{ ok: false, reason }} />));
  const alert = host.querySelector('[role="alert"]')!;
  expect(alert.textContent).toContain(reason);
  expect(alert.textContent).toContain("不可用");
  expect(alert.textContent).not.toContain("{{");
});
