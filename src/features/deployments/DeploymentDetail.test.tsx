import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { DeploymentDetail } from "../../app/pages/DeploymentDetail";
import { i18n } from "../../shared/i18n/i18n";
import { artifactDiff } from "./deployment";
vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value }: { value: string }) => <pre data-editor-value>{value}</pre> }));
let root: Root, host: HTMLDivElement, state: string, calls: string[];
const mount = async () => { await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={new Set(["deployments.apply"])} />)); };
const tick = async (ms: number) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
beforeEach(() => {
  localStorage.setItem("ui.locale", "zh-CN");
  void i18n.changeLanguage("zh-CN");
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

it("historical rendering stays fixed while live application polling restores actions", async () => {
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    calls.push(url);
    const row = url.includes("deployment-revisions")
      ? { id: "10", deployment_id: "1", number: 2, state: "succeeded", rendered: "historical-content", action: "apply" }
      : { id: "1", name: "web", state, mode: "native", target_ref: "2", target_name: "Linux host", version: 3 };
    return { ok: true, json: async () => row };
  }));
  await act(async () => root.render(<DeploymentDetail workspace="ops" revisionId="10" permissions={new Set()} />));
  const preview = () => [...host.querySelectorAll("button")].find(button => button.textContent === "预览目标差异")!;
  expect(preview().disabled).toBe(true);
  const before = calls.length;
  await tick(1200);
  expect(calls.length).toBeGreaterThan(before);
  state = "succeeded";
  await tick(1200);
  expect(preview().disabled).toBe(false);
  const count = calls.length;
  await tick(6000);
  expect(calls).toHaveLength(count);
  const render = [...host.querySelectorAll("button")].find(button => button.textContent === "查看渲染产物")!;
  expect(render).toBeDefined();
  await act(async () => render.click());
  expect(host.querySelector("[data-editor-value]")?.textContent).toBe("historical-content");
  expect(calls).toHaveLength(count);
  expect(host.textContent).toContain("Linux host");
});
