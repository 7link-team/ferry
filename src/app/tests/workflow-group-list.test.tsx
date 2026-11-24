import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { listConfigs } from "../resources";
import { WorkflowList } from "../pages/WorkflowList";

vi.mock("../pages/GroupSidebar", () => ({
  GroupSidebar: ({ active, onSelect }: { active: string; onSelect: (id: string) => void }) =>
    <div data-testid="groups" data-active={active}>
      <button onClick={() => onSelect("0")}>Ungrouped</button>
      <button onClick={() => onSelect("42")}>Group</button>
    </div>,
}));
vi.mock("../resources/ResourceTable", () => ({
  ResourceTable: ({ config }: { config: { fixed?: Record<string, string>; filters?: { key: string }[]; columns: { key: string }[] } }) =>
    <div data-testid="table" data-fixed={JSON.stringify(config.fixed)} data-filters={config.filters?.map(filter => filter.key).join(",")} data-columns={config.columns.map(column => column.key).join(",")} />,
}));

let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/en-US/workspaces/ops/workflows?page=3&at=240");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

it("selects workflow groups through the URL and resets table pagination", async () => {
  const config = listConfigs("ops").workflows!;
  await act(async () => root.render(<WorkflowList config={config} workspace="ops" permissions={new Set(["workflow-groups.read"])} />));
  expect(host.querySelector('[data-testid="groups"]')).not.toBeNull();
  expect(host.querySelector('[data-testid="table"]')?.getAttribute("data-filters")).not.toContain("group_id");
  expect(host.querySelector('[data-testid="table"]')?.getAttribute("data-columns")).toContain("group_name");
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  expect(location.search).toBe("?group_id=0");
  expect(host.querySelector('[data-testid="table"]')?.getAttribute("data-fixed")).toContain('"group_id":"0"');
  await act(async () => host.querySelectorAll<HTMLButtonElement>("button")[1]!.click());
  expect(new URLSearchParams(location.search).get("group_id")).toBe("42");
});

it("keeps the group filter when group read permission is absent", async () => {
  const config = listConfigs("ops").workflows!;
  await act(async () => root.render(<WorkflowList config={config} workspace="ops" permissions={new Set()} />));
  expect(host.querySelector('[data-testid="groups"]')).toBeNull();
  expect(host.querySelector('[data-testid="table"]')?.getAttribute("data-filters")).toContain("group_id");
});
