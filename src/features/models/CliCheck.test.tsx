import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { CliCheck } from "./CliCheck";

let root: Root, host: HTMLDivElement;
let requests: { url: string; body: string | undefined }[];
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, body: init.body as string });
    return { ok: true, status: 200, json: async () => ({ items: [
      { host_id: "1", host_name: "ASPEED", ok: true, executable: "C:/npm/codex.cmd", user: "FullStack", version: "0.168.0", authentication: "logged_in" },
      { host_id: "2", host_name: "Mac mini", ok: false, user: "aspeed", reason: "Runner 执行账号 aspeed 下未找到 codex" },
    ] }) };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("checks each execution host and says where the CLI was found or why not", async () => {
  await act(async () => root.render(<CliCheck workspace="ops" runtime="codex" hosts={["1", "2"]} />));
  const run = [...host.querySelectorAll("button")].find(b => b.textContent === "立即检测")!;
  await act(async () => run.click());
  expect(requests[0]!.url).toBe("/api/workspaces/ops/ai-models/cli-checks");
  expect(JSON.parse(requests[0]!.body!)).toEqual({ runtime: "codex", hosts: ["1", "2"] });
  const text = host.textContent ?? "";
  expect(text).toContain("C:/npm/codex.cmd");
  expect(text).toContain("FullStack");
  expect(text).toContain("已登录");
  expect(text).toContain("Runner 执行账号 aspeed 下未找到 codex");
});

it("cannot run without a CLI tool or the probe permission", async () => {
  await act(async () => root.render(<CliCheck workspace="ops" runtime="" hosts={[]} />));
  expect([...host.querySelectorAll("button")].find(b => b.textContent === "立即检测")!.disabled).toBe(true);
  await act(async () => root.render(<CliCheck workspace="ops" runtime="codex" hosts={[]} disabled />));
  expect([...host.querySelectorAll("button")].find(b => b.textContent === "立即检测")!.disabled).toBe(true);
});
