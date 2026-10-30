import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceTable } from "./ResourceTable";
import { navigate } from "../router";
import type { ListConfig } from "./dsl";
import { listResource } from "../resource";

vi.mock("../router", () => ({
  useRoute: () => ({ workspace: "ops", group: "host", item: "hosts", mode: "list" }),
  useQueryValue: (_key: string, fallback: unknown) => [fallback, () => {}],
  useScrollSync: () => ({ current: null }),
  href: () => "/detail", navigate: vi.fn(),
}));
vi.mock("../resource", async original => ({ ...await original<typeof import("../resource")>(),
  listResource: vi.fn(async () => ({ items: [{ id: "1", name: "Alpha" }, { id: "2", name: "Beta" }], total: 2, page: 1, page_size: 20 })),
}));
let root: Root;
let element: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); vi.clearAllMocks(); });
it("shows refresh failures even when a remote page already contains rows, then recovers", async () => {
  const first = { items: [{ id: "1", name: "Repository" }], total: 0, page: 1, page_size: 50, has_more: true };
  vi.mocked(listResource).mockResolvedValueOnce(first);
  const config: ListConfig<{ id: string; name: string }> = {
    resource: "connection-repositories", pagination: "next", rowKey: row => row.id,
    columns: [{ key: "name", label: "仓库名称" }], toolbar: { create: false },
  };
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  const next = () => element.querySelector<HTMLButtonElement>('button[aria-label="下一页"]')!;
  expect(next().disabled).toBe(false);
  expect(element.textContent).not.toContain("共 0 条");
  const refresh = () => [...element.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.trim() === "刷新")!;
  vi.mocked(listResource).mockRejectedValueOnce(new Error("Provider HTTP 403"));
  await act(async () => refresh().click());
  expect(element.querySelector('[role="alert"]')?.textContent).toBe("Provider HTTP 403");
  expect(next().disabled).toBe(true);
  vi.mocked(listResource).mockResolvedValueOnce({ ...first, has_more: false });
  await act(async () => refresh().click());
  expect(element.querySelector('[role="alert"]')).toBeNull();
  expect(next().disabled).toBe(true);
});
it("isolates pending actions to their own row/button and allows concurrent probes and navigation", async () => {
  const finish = new Map<string, () => void>();
  const action = vi.fn((row: { id: string }) => new Promise<void>(done => finish.set(row.id, done)));
  const config: ListConfig<{ id: string; name: string }> = {
    resource: "hosts", rowKey: row => row.id, columns: [{ key: "name", label: "名称" }],
    toolbar: { create: false, import: false, export: false },
    actions: [{ label: "测试连接", action }, { label: "查看", to: row => ({ id: row.id, mode: "view" }) }],
  };
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  const probes = () => [...element.querySelectorAll<HTMLButtonElement>('button[aria-label="测试连接"]')];
  await act(async () => probes()[0]!.click());
  expect(probes()[0]!.disabled).toBe(true);
  expect(probes()[0]!.getAttribute("aria-busy")).toBe("true");
  expect(probes()[1]!.disabled).toBe(false);
  expect(element.querySelectorAll('a[aria-label="查看"]')).toHaveLength(2);
  await act(async () => element.querySelector<HTMLElement>('a[aria-label="查看"]')!.click());
  expect(navigate).toHaveBeenCalledTimes(1);
  await act(async () => probes()[1]!.click());
  expect(action).toHaveBeenCalledTimes(2);
  await act(async () => finish.get("1")!());
  expect(probes()[0]!.disabled).toBe(false);
  expect(probes()[1]!.disabled).toBe(true);
  await act(async () => finish.get("2")!());
  expect(probes().every(button => !button.disabled)).toBe(true);
});
