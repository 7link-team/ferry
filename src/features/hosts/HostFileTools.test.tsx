import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { HostFileTools } from "./HostFileTools";

vi.mock("../../shared/api/api", () => ({
  api: vi.fn(),
  errorText: (cause: unknown) => (cause instanceof Error ? cause.message : String(cause)),
}));
const calls = vi.mocked(api);
const t = (key: string) => commonText(`resource-domain:fileTools.${key}`);
let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  calls.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
const check = async () => {
  const button = [...host.querySelectorAll("button")].find((b) => b.textContent === t("check"))!;
  await act(async () => button.click());
};

it("checks a Runner's file-production tools on request and lists them", async () => {
  calls.mockResolvedValueOnce({ tools: ["python 3.12.1 (python-pptx, matplotlib)", "ffmpeg 6.1", "claude skills: pptx"], write_workdir: true } as never);
  await act(async () => root.render(<HostFileTools workspace="7" hostId="42" />));
  expect(calls).not.toHaveBeenCalled();
  await check();
  expect(calls).toHaveBeenCalledWith("/workspaces/7/nodes/42/file-tools");
  const items = [...host.querySelectorAll("li")].map((li) => li.textContent);
  expect(items).toEqual(["python 3.12.1 (python-pptx, matplotlib)", "ffmpeg 6.1", "claude skills: pptx"]);
  expect(host.textContent).not.toContain(t("upgrade"));
});

it("says when nothing is found, when the Runner is too old, and shows errors as an alert", async () => {
  calls.mockResolvedValueOnce({ tools: [], write_workdir: false } as never);
  await act(async () => root.render(<HostFileTools workspace="7" hostId="42" />));
  await check();
  expect(host.textContent).toContain(t("none"));
  expect(host.textContent).toContain(t("upgrade"));
  calls.mockRejectedValueOnce(new Error("主机离线"));
  await check();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("主机离线");
});
