import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { i18n, workflowText as t } from "../../shared/i18n/i18n";
import { WorkflowNodeParameters, WorkflowRunInputs, validRunInputs } from "./WorkflowNodeParameters";
import { emptyGraph, newNode, type GraphNode } from "./graph";

let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
async function input(label: string, value: string) {
  const element = [...host.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input,select")].find(item => item.getAttribute("aria-label") === label)!;
  expect(element, label).toBeDefined();
  await act(async () => {
    Object.getOwnPropertyDescriptor(element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}
async function click(text: string) {
  const element = [...host.querySelectorAll("button")].find(item => item.textContent?.trim() === text)!;
  expect(element, text).toBeDefined(); await act(async () => element.click());
}
it("edits nested run objects and array items through ordinary controls and retains schema validation", async () => {
  const schema = { type: "object", required: ["profile"], properties: { profile: { type: "object", required: ["age"], properties: { age: { type: "integer" }, enabled: { type: "boolean" }, names: { type: "array", items: { type: "string" } } } } } };
  let current: Record<string, unknown> = {};
  function Harness() { const [value, setValue] = useState(current); current = value; return <WorkflowRunInputs schema={schema} value={value} change={setValue} invalid={() => {}} />; }
  await act(async () => root.render(<Harness />));
  expect(validRunInputs(schema, current)).toBe(false);
  await input("profile.age", "12"); await input("profile.enabled", "false");
  await click("添加 profile.names");
  await input("profile.names.0", "Ada");
  expect(current).toEqual({ profile: { age: 12, enabled: false, names: ["Ada"] } });
  expect(validRunInputs(schema, current)).toBe(true);
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.getAttribute("aria-label") === `${t("delete")} profile.names.0`)!.click());
  expect(current).toEqual({ profile: { age: 12, enabled: false, names: [] } });
  expect(host.querySelectorAll("details[open]")).toHaveLength(0);
  await input("profile.age", "1.5"); expect(validRunInputs(schema, current)).toBe(false);
});
it("edits object and list literal node values without JSON and disables all ordinary mutation in read-only mode", async () => {
  let current: Record<string, unknown> = { fields: [{ path: "result", value: { title: "before", tags: ["one"] } }], keep_other_fields: true };
  const node: GraphNode = { id: "set", type: "data.set", type_version: 1, name: "set", position: [0, 0], params: current };
  function Harness({ readOnly = false }: { readOnly?: boolean }) { const [value, setValue] = useState(current); current = value; return <WorkflowNodeParameters node={{ ...node, params: value }} readOnly={readOnly} update={setValue} invalid={() => {}} />; }
  await act(async () => root.render(<Harness />));
  // Use the value path rather than textarea contents: ordinary editors must update stored structured data.
  const title = [...host.querySelectorAll<HTMLInputElement>("input")].find(item => item.value === "before")!;
  await input(title.getAttribute("aria-label")!, "after");
  expect((current.fields as { value: unknown }[])[0]!.value).toEqual({ title: "after", tags: ["one"] });
  await act(async () => root.render(<Harness readOnly />));
  expect([...host.querySelectorAll("input,select,textarea")].every(item => (item as HTMLInputElement).disabled || (item as HTMLInputElement).readOnly)).toBe(true);
  expect([...host.querySelectorAll("button")].every(button => button.disabled)).toBe(true);
});
it("generates platform data path parameters from the selected GET operation and states where it runs", async () => {
  let current: Record<string, unknown> = newNode(emptyGraph(), "ferry.request", "read").params;
  expect(current).toMatchObject({ operation: "", path_params: {}, query: {}, split_items: true, all_pages: false, max_pages: 50 });
  const node: GraphNode = { id: "read", type: "ferry.request", type_version: 1, name: "read", position: [0, 0], params: current };
  function Harness({ readOnly = false }: { readOnly?: boolean }) { const [value, setValue] = useState(current); current = value; return <WorkflowNodeParameters node={{ ...node, params: value }} readOnly={readOnly} update={setValue} invalid={() => {}} />; }
  await act(async () => root.render(<Harness />));
  expect(host.textContent).toContain("执行位置：平台服务器内（不经主机）");
  const operation = host.querySelector<HTMLInputElement>(`input[aria-label="${t("platform.operation")}"]`)!;
  await act(async () => operation.focus());
  const options = [...host.querySelectorAll('[role="option"]')].map(option => option.textContent);
  expect(options).toContain("/code-insights/{id}/reports/{kind}");
  // Only read-only catalogue templates are offered (no write-only template such as sync).
  expect(options).not.toContain("/code-insights/{id}/sync");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>('[role="option"]')].find(option => option.textContent === "/code-insights/{id}/reports/{kind}")!.click());
  expect(current).toMatchObject({ operation: "/code-insights/{id}/reports/{kind}", path_params: { id: "", kind: "" } });
  expect(host.textContent).toContain(`${t("platform.permission")}: code-insights.read`);
  await input(`${t("platform.pathParam")} · id`, "42");
  await input(`${t("platform.pathParam")} · kind`, "contributors");
  expect(current.path_params).toEqual({ id: "42", kind: "contributors" });
  // Switching operation keeps placeholders that still exist and clears the rest.
  await input(t("platform.operation"), "/code-insights/{id}");
  expect(current.path_params).toEqual({ id: "42" });
  expect(host.querySelector(`input[aria-label="${t("platform.pathParam")} · kind"]`)).toBeNull();
  await input(t("platform.operation"), "/code-insights/{id}/nope");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe(t("platform.unknownOperation"));
  await click(`${t("addRow")} ${t("platform.query")}`);
  expect(current.query).toEqual({ field: "" });
  expect(host.querySelector(`input[aria-label="${t("platform.maxPages")}"]`)).toBeNull();
  await act(async () => root.render(<Harness readOnly />));
  expect(host.textContent).toContain("执行位置：平台服务器内（不经主机）");
  expect([...host.querySelectorAll("input")].every(item => item.disabled || item.readOnly)).toBe(true);
});
