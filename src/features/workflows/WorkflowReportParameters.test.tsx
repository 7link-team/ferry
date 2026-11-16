import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { i18n, workflowText as t } from "../../shared/i18n/i18n";
import { WorkflowNodeParameters } from "./WorkflowNodeParameters";
import { emptyGraph, newNode, type GraphNode } from "./graph";
import { nodeTitle } from "./WorkflowNodePicker";

let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

function setValue(element: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  const proto = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
}

it("creates a report node fed by the previous step's JSON and edits its content", async () => {
  let current: Record<string, unknown> = newNode(emptyGraph(), "report.render", "report").params;
  expect(current).toMatchObject({ format: "pdf", title: "={{ $json.title }}", sections: "={{ $json.sections }}", output_field: "file" });
  const node: GraphNode = { id: "report", type: "report.render", type_version: 1, name: "report", position: [0, 0], params: current };
  function Harness({ readOnly = false }: { readOnly?: boolean }) {
    const [value, setValue] = useState(current); current = value;
    return <WorkflowNodeParameters node={{ ...node, params: value }} readOnly={readOnly} update={setValue} invalid={() => {}} />;
  }
  await act(async () => root.render(<Harness />));
  expect(host.textContent).toContain(t("report.help"));
  expect([...host.querySelectorAll("option")].map(o => o.textContent)).toContain("PDF");
  await act(async () => setValue(host.querySelector<HTMLInputElement>(`input[aria-label="${t("report.title")}"]`)!, "Codeup 周报"));
  expect(current.title).toBe("Codeup 周报");
  // Sections switch between an expression and hand-edited rows.
  const sections = host.querySelector<HTMLInputElement>(`input[aria-label="${t("report.sections")}"]`)!;
  expect(sections.value).toBe("={{ $json.sections }}");
  const manual = [...host.querySelectorAll("button")].filter(b => b.textContent === t("report.editManually"));
  await act(async () => manual[1]!.click());
  expect(current.sections).toEqual([]);
  await act(async () => setValue(host.querySelector<HTMLSelectElement>(`select[aria-label="${t("report.language")}"]`)!, "en"));
  expect(current.language).toBe("en");
  await act(async () => root.render(<Harness readOnly />));
  expect([...host.querySelectorAll("input,textarea")].every(i => (i as HTMLInputElement).readOnly)).toBe(true);
  expect(host.querySelectorAll("button").length).toBe(0);
});

it("offers the report node in the picker with a localized title", () => {
  expect(nodeTitle("report.render")).toBe(t("reportRender"));
});
