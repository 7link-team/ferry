import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { listResource } from "../../shared/api/resource";
import { commonText, i18n } from "../../shared/i18n/i18n";
import type { ListConfig } from "../../shared/resources/types";
import { navigate } from "../router";
import { ResourceTable } from "./ResourceTable";

vi.mock("../router", () => ({
  useRoute: () => ({ workspace: "ops", group: "workflow", item: "workflows", mode: "list" }),
  useQueryValue: (_key: string, fallback: unknown) => [fallback, () => {}],
  useScrollSync: () => ({ current: null }),
  href: () => "/detail", navigate: vi.fn(),
}));
vi.mock("../../shared/api/resource", async original => ({ ...await original<typeof import("../../shared/api/resource")>(),
  listResource: vi.fn(async () => ({ items: [{ id: "1", name: "Build API", group: "Backend" }, { id: "2", name: "Ship app", group: "Mobile" }], total: 5, page: 1, page_size: 20 })),
}));

let root: Root;
let element: HTMLDivElement;
beforeEach(() => {
  void i18n.changeLanguage("en-US");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  // A phone-sized screen.
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: query.includes("max-width"), media: query, addEventListener() {}, removeEventListener() {} }));
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); vi.unstubAllGlobals(); vi.clearAllMocks(); });

const config: ListConfig<{ id: string; name: string; group: string }> = {
  resource: "workflows", rowKey: row => row.id,
  columns: [{ key: "name", label: "Name" }, { key: "group", label: "Group" }],
  filters: [{ key: "q", label: "Name", type: "text" }, { key: "state", label: "State", type: "select", options: [{ value: "active", label: "Active" }] }],
  actions: [{ label: "View", to: row => ({ id: row.id, mode: "view" }) }, { label: "Edit", to: row => ({ id: row.id, mode: "edit" }) }],
};

it("shows phone rows as a list with a search box, filters on demand and load more", async () => {
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set()} />));
  expect(element.querySelector("table")).toBeNull();
  const rows = element.querySelectorAll("[data-phone-row]");
  expect(rows).toHaveLength(2);
  expect(rows[0]!.textContent).toContain("Build API");
  expect(rows[0]!.textContent).toContain("Backend");
  // One search box; the other filters stay hidden until asked for.
  expect(element.querySelector('[data-phone-search] input[type="search"]')).not.toBeNull();
  expect(element.querySelector("select, [role=combobox]")).toBeNull();
  const filters = [...element.querySelectorAll("button")].find(button => button.textContent?.includes(commonText("table.filters")))!;
  await act(async () => filters.click());
  expect(filters.getAttribute("aria-expanded")).toBe("true");
  // Desktop-only controls are gone; more rows load in place of pagination.
  expect(element.textContent).not.toContain(commonText("table.columns"));
  expect([...element.querySelectorAll("button")].some(button => button.textContent === commonText("table.loadMore"))).toBe(true);
  // Tapping a row opens it.
  await act(async () => (rows[1]!.querySelector("button") as HTMLButtonElement).click());
  expect(navigate).toHaveBeenCalledWith(expect.objectContaining({ id: "2", mode: "view" }), {});
  expect(listResource).toHaveBeenCalled();
});
