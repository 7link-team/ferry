import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { DeploymentDetail } from "./DeploymentDetail";
import { artifactDiff } from "../deployment";
let root: Root, host: HTMLDivElement, state: string, calls: string[];
const mount = async () => { await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={new Set(["deployments.apply"])} />)); };
const tick = async (ms: number) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
beforeEach(() => {
  vi.useFakeTimers(); Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "hidden", { configurable: true, value: false });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  state = "running"; calls = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string) => { calls.push(url); return { ok: true, json: async () => ({ id: "1", name: "web", state, target_ref: "2", version: 1 }) }; }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
it.each(["idle", "succeeded", "failed", "needs-review"])("%s stops periodic polling", async (terminal) => {
  await mount(); state = terminal; await tick(1200); const n = calls.length; await tick(6000); expect(calls).toHaveLength(n);
});
it("hidden pauses and visible resumes", async () => {
  await mount(); const n = calls.length;
  Object.defineProperty(document, "hidden", { value: true }); await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  await tick(5000); expect(calls).toHaveLength(n);
  Object.defineProperty(document, "hidden", { value: false }); await act(async () => document.dispatchEvent(new Event("visibilitychange")));
  expect(calls.length).toBe(n + 1);
});
it("execution requires preview and is blocked while uncertain", async () => {
  state = "needs-review"; await mount();
  const apply = [...host.querySelectorAll("button")].find((b) => b.textContent === "执行部署")!;
  expect(apply.disabled).toBe(true); expect(host.textContent).toContain("远端命令可能仍在运行");
});
it("diff retains reordered lines as a change and exact equality as unchanged", () => {
  expect(artifactDiff("a\nb\n", "b\na\n")).toContain("- a\n- b\n+ b\n+ a");
  expect(artifactDiff("a\n", "a\n")).toBe("产物内容一致。");
});
