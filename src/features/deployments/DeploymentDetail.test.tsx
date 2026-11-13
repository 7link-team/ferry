import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { DeploymentDetail } from "../../app/pages/DeploymentDetail";
import { commonText, i18n } from "../../shared/i18n/i18n";
import { artifactDiff, revisionActionLabel } from "./deployment";
import { deploymentsLists } from "./resource-config";
vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value }: { value: string }) => <pre data-editor-value>{value}</pre> }));
let root: Root, host: HTMLDivElement, state: string, calls: string[];
const mount = async () => { await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={new Set(["deployments.apply"])} />)); };
const tick = async (ms: number) => { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); };
beforeEach(() => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/deployments/1");
  localStorage.setItem("ui.locale", "zh-CN");
  void i18n.changeLanguage("zh-CN");
  vi.useFakeTimers(); Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "hidden", { configurable: true, value: false });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  state = "running"; calls = [];
  vi.stubGlobal("fetch", vi.fn(async (url: string) => { calls.push(url); return { ok: true, json: async () => ({ id: "1", name: "web", state, target_ref: "2", version: 1 }) }; }));
});

it("loads current health only on the authorized health tab and keeps the application title", async () => {
  state = "idle";
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    calls.push(url);
    return { ok: true, json: async () => url.includes("application-health")
      ? { id: "1", name: "web", health: "unknown", entry_state: "unknown", healthy_instances: 0, expected_instances: 0, unknown_instances: 0, unmonitored_hosts: 1, services: [], instances: [], checks: [] }
      : { id: "1", name: "web", state, target_ref: "2", version: 1 } };
  }));
  await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={new Set(["deployments.read", "alerts.read", "alerts.create"])} />));
  expect(calls.some(url => url.includes("application-health"))).toBe(false);
  const tab = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(tab => tab.textContent === commonText("monitoring:healthTab"))!;
  expect(tab).toBeDefined();
  await act(async () => tab.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
  expect(location.search).toContain("tab=health");
  expect(calls.some(url => url.includes("application-health/1"))).toBe(true);
  expect(host.querySelector("h2")?.textContent).toBe("web");
  const add = [...host.querySelectorAll<HTMLAnchorElement>("a")].find(link => link.textContent === "添加健康检查")!;
  expect(add.href).toContain("/health-checks/new?deployment_id=1");
  const policy = [...host.querySelectorAll<HTMLAnchorElement>("a")].find(link => link.textContent === commonText("monitoring:createPolicy"))!;
  expect(policy.href).toContain("target_kind=application-health");
  expect(policy.href).toContain("target_ref=1");
  await tick(5000);
  expect(calls.filter(url => url.includes("application-health/1")).length).toBeGreaterThan(1);
});

it("does not expose health controls or make health requests without alerts.read", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/deployments/1?tab=health");
  state = "idle"; await mount();
  expect(host.querySelector('[role="tablist"]')).toBeNull();
  expect(calls.some(url => url.includes("application-health"))).toBe(false);
});

it("shows health list columns only with permission and preserves zero replica counts", () => {
  const config = (permissions: string[]) => deploymentsLists({ workspace: "ops", permissions: new Set(permissions) } as never).deployments!;
  expect(config([]).columns.some(column => column.key === "health")).toBe(false);
  const columns = config(["alerts.read"]).columns;
  expect(columns.some(column => column.key === "health")).toBe(true);
  expect(columns.find(column => column.key === "health_replicas")!.render!({ healthy_instances: 0, expected_instances: 0 })).toBe("0 / 0");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
it.each(["auto-replace", "manual-replace", "restart", "rollback", "apply"])("revision_title_labels_%s", async action => {
  state = "succeeded";
  vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => ({ id: "1", name: "web", state, version: 1, latest_revision: { id: "9", number: 3, action, state, started_at: 0, finished_at: 0 } }) })));
  await mount(); expect(host.querySelector("h3")?.textContent).toContain(revisionActionLabel(action));
  expect(revisionActionLabel(action)).not.toMatch(/^failover\./);
});
it("idle_blueprint_refreshes_failover_summary_while_visible", async () => {
  state = "idle";
  vi.stubGlobal("fetch", vi.fn(async url => { calls.push(String(url)); return { ok: true, json: async () => String(url).includes("/events") ? { items: [], total: 0 } : { id: "1", name: "web", state, version: 1, target_kind: "blueprint", mode: "docker", failover: { mode: "auto", host_down_after: 90, fences: [], attention: [] } } }; }));
  await mount(); const count = calls.filter(url => url.endsWith("/deployments/1")).length;
  await tick(10_000); expect(calls.filter(url => url.endsWith("/deployments/1"))).toHaveLength(count + 1);
  expect(host.textContent).toContain(commonText("resource-domain:failover.timeline"));
});
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

it("blueprint deployments show topology and the spec editor; others keep the existing page", async () => {
  let row: Record<string, unknown> = { id: "1", name: "nats", state: "idle", mode: "docker", target_kind: "blueprint", target_ref: "0", version: 2, current_revision: "0", manifest: "",
    blueprint_binding: { driver: "docker", roles: { nats: [{ hostId: "5", address: "10.0.0.5" }] } }, deploy_spec: "apiVersion: ferry/deploy-v1\n" };
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    const body = url.includes("application-health") ? { placement: [] } : url.includes("/nodes") ? { items: [{ id: "5", name: "host-e" }], total: 1, page: 1, page_size: 200 } : row;
    return { ok: true, json: async () => body };
  }));
  const permissions = new Set(["deployments.update", "deployments.read", "alerts.read", "hosts.read"]);
  await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={permissions} />));
  for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); });
  expect(host.querySelector('[data-topology-instance="nats-0"]')?.textContent).toContain("host-e");
  expect([...host.querySelectorAll("button")].some(b => b.textContent === "编辑部署规格")).toBe(true);
  await act(async () => root.unmount()); root = createRoot(host);
  row = { id: "1", name: "web", state: "idle", mode: "native", target_kind: "host", target_ref: "2", version: 1, current_revision: "0", deploy_spec: null };
  await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={permissions} />));
  for (let i = 0; i < 6; i++) await act(async () => { await Promise.resolve(); });
  expect(host.querySelector("[data-topology-role]")).toBeNull();
  expect([...host.querySelectorAll("button")].some(b => b.textContent === "编辑部署规格")).toBe(false);
});
