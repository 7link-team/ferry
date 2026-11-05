import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { EditorFormActionsContext } from "../../shared/forms/auto-form/form-actions";
import { i18n, workflowText as t } from "../../shared/i18n/i18n";
import { emptyGraph, newNode, type Graph } from "./graph";
import { WorkflowNodeInspector } from "./WorkflowNodeInspector";
import { WorkflowDataView } from "./WorkflowDataView";
import { fieldExpression, nodeModes, canPinNode } from "./workflow-data";

vi.mock("../../shared/api/api", () => ({
  api: vi.fn(),
  errorText: (cause: unknown) =>
    cause instanceof Error ? cause.message : String(cause),
}));
const calls = vi.mocked(api);
let host: HTMLDivElement, root: Root, current: Graph;
const hash = "a".repeat(64);
const sample = { profile: { name: "Ada" }, "a.b": [{ value: 7 }] };
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  const node = newNode(emptyGraph(), "data.set", "Assign");
  node.params = {
    keep_other_fields: true,
    fields: [{ path: "result", value: "before" }],
  };
  current = { ...emptyGraph(), nodes: [node] };
  calls.mockReset();
  calls.mockImplementation(async (path) => {
    if (path.includes("/executions?"))
      return {
        items: [{ id: "91", number: 1, state: "succeeded", revision_id: "15" }],
      };
    if (path.endsWith("/node-runs"))
      return {
        items: [{ id: "92", node_id: node.id, attempt: 1, state: "succeeded" }],
      };
    if (path.endsWith("/data"))
      return {
        input: {
          items: [{ json: sample }],
          sources: [
            {
              node: "start",
              iteration_hash: hash,
              port: "main",
              index: 0,
              items: [{ json: sample }],
            },
          ],
        },
        output: {
          ports: [
            { name: "main", index: 0, items: [{ json: { result: "before" } }] },
          ],
        },
        resolved_params: [node.params],
        revision_id: "15",
        frozen_at: 1000,
        state: "succeeded",
        end_reason: "",
      };
    if (path.includes("/revisions/"))
      return { definition: JSON.stringify({ ...current, nodes: [node] }) };
    if (path.endsWith("/expression-previews"))
      return {
        items: [{ ...node.params, fields: [{ path: "result", value: "Ada" }] }],
      };
    return { version: 1, definition: JSON.stringify(current) };
  });
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});
const dialog = () => document.querySelector('[role="dialog"]')!;
const button = (name: string) =>
  [...dialog().querySelectorAll<HTMLButtonElement>("button")].find(
    (item) =>
      item.textContent?.trim() === name ||
      item.getAttribute("aria-label") === name,
  )!;
async function click(name: string) {
  const item = button(name);
  expect(item, name).toBeDefined();
  await act(async () => item.click());
}
function Harness({
  readOnly = false,
  save = () => {},
}: {
  readOnly?: boolean;
  save?: () => void;
}) {
  const [graph, setGraph] = useState(current);
  current = graph;
  return (
    <EditorFormActionsContext.Provider
      value={{ save, saving: false, blocked: readOnly }}
    >
      <WorkflowNodeInspector
        workspace="11"
        workflow="7"
        graph={graph}
        node={graph.nodes[0]!}
        draftRevision="15"
        readOnly={readOnly}
        canRun={!readOnly}
        inputFields={[]}
        update={(value) =>
          setGraph({ ...graph, nodes: [{ ...graph.nodes[0]!, ...value }] })
        }
        invalid={() => {}}
        changeGraph={setGraph}
        close={() => {}}
        select={() => {}}
        duplicate={() => {}}
        remove={() => {}}
        run={() => {}}
      />
    </EditorFormActionsContext.Provider>
  );
}
it("renders authenticated historical inputs, editable parameters and actual output in three independent panes", async () => {
  await act(async () => root.render(<Harness />));
  for (const key of ["input", "parameters", "output"])
    expect(
      dialog().querySelector(
        `section[aria-label="${t(`inspector.${key}` as Parameters<typeof t>[0])}"]`,
      ),
    ).not.toBeNull();
  expect(dialog().textContent).toContain("Ada");
  expect(dialog().textContent).toContain("before");
  expect(
    dialog().querySelectorAll("details[open]").length,
  ).toBeGreaterThanOrEqual(1);
  expect(button(t("inspector.resolve")).disabled).toBe(false);
});
it("inserts nested and escaped field references into the existing controlled form and previews without executing", async () => {
  const save = vi.fn();
  await act(async () => root.render(<Harness save={save} />));
  const field = [...dialog().querySelectorAll<HTMLInputElement>("input")].find(
    (item) => item.value === "before",
  )!;
  await act(async () => field.focus());
  await click(`${t("inspector.reference")} ["profile","name"]`);
  expect(
    (current.nodes[0]!.params.fields as { value: string }[])[0]!.value,
  ).toBe('={{ $json["profile"]["name"] }}');
  expect(dialog().textContent).toContain(t("inspector.historical"));
  await click(t("inspector.resolve"));
  expect(dialog().textContent).toContain("Ada");
  const request = calls.mock.calls.find(([path]) =>
    path.endsWith("/expression-previews"),
  )!;
  const body = JSON.parse(String(request[1]?.body));
  expect(body.data.items[0].json).toEqual(sample);
  expect(body.data.upstream[0].iteration_hash).toBe(hash);
  expect(
    calls.mock.calls.some(
      ([path, options]) =>
        path.endsWith("/executions") && options?.method === "POST",
    ),
  ).toBe(false);
  await click("保存");
  expect(save).toHaveBeenCalledOnce();
  expect(fieldExpression(["a.b", 0, 'quote"key'])).toBe(
    '={{ $json["a.b"][0]["quote\\"key"] }}',
  );
});
it("inserts a reference into an existing readable field-path control without nesting expressions", async () => {
  current.nodes[0]!.params.fields = [{path:"result", value:"={{ $json.profile.name }}"}];
  await act(async () => root.render(<Harness />));
  const field = [...dialog().querySelectorAll<HTMLInputElement>("input")].find(item => item.value === "profile.name")!;
  expect(field).toBeDefined();
  await act(async () => field.focus());
  await click(`${t("inspector.reference")} ["profile","name"]`);
  expect((current.nodes[0]!.params.fields as {value:string}[])[0]!.value).toBe('={{ $json["profile"]["name"] }}');
  expect(field.value).toBe("profile.name");
});
it("offers supported modes only, preserves other settings and pins actual output without paired execution references", async () => {
  await act(async () => root.render(<Harness />));
  await click(t("inspector.settings"));
  const select = dialog().querySelector<HTMLSelectElement>(
    `select[aria-label="${t("inspector.mode")}"]`,
  )!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLSelectElement.prototype,
      "value",
    )!.set!.call(select, "bypass");
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  expect(current.nodes[0]!.settings?.mode).toBe("bypass");
  // Return to the matching execution before pinning; changed settings cannot claim old output is current.
  await act(async () => {
    Object.getOwnPropertyDescriptor(
      HTMLSelectElement.prototype,
      "value",
    )!.set!.call(select, "active");
    select.dispatchEvent(new Event("change", { bubbles: true }));
  });
  expect(button(t("inspector.pin")).disabled).toBe(true);
  expect(nodeModes(newNode(emptyGraph(), "core.if", "If"))).not.toContain(
    "bypass",
  );
  expect(canPinNode(newNode(emptyGraph(), "core.if", "If"))).toBe(false);
  expect(canPinNode(newNode(emptyGraph(), "core.stop", "Stop"))).toBe(false);
});
it("pins and unpins the selected execution output", async () => {
  await act(async () => root.render(<Harness />));
  await click(t("inspector.settings"));
  await click(t("inspector.pin"));
  expect(current.pin_data).toEqual({
    [current.nodes[0]!.id]: [{ json: { result: "before" } }],
  });
  await click(t("inspector.unpin"));
  expect(current.pin_data).toEqual({});
});
it("keeps read-only inspection available while disabling settings, reference insertion and preview", async () => {
  await act(async () => root.render(<Harness readOnly />));
  expect(button(t("inspector.resolve")).disabled).toBe(true);
  expect(button("保存")).toBeUndefined();
  await click(t("inspector.settings"));
  expect(
    [
      ...dialog()
        .querySelector(`section[aria-label="${t("inspector.parameters")}"]`)!
        .querySelectorAll<
          HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
        >("input,textarea,select"),
    ].every((item) => item.disabled || ("readOnly" in item && item.readOnly)),
  ).toBe(true);
});
it("paginates data without dropping full item JSON and translates inspector labels in all locales", async () => {
  await act(async () =>
    root.render(
      <WorkflowDataView
        items={Array.from({ length: 21 }, (_, index) => ({ json: { index } }))}
      />,
    ),
  );
  const find = (name: string) =>
    [...host.querySelectorAll<HTMLButtonElement>("button")].find(
      (item) =>
        item.textContent === name || item.getAttribute("aria-label") === name,
    )!;
  await act(async () => find(t("inspector.table")).click());
  expect(host.querySelectorAll("tbody tr")).toHaveLength(20);
  await act(async () => find(t("inspector.nextPage")).click());
  expect(host.querySelectorAll("tbody tr")).toHaveLength(1);
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    await i18n.changeLanguage(locale);
    for (const key of [
      "inspector.open",
      "inspector.mode.mute",
      "inspector.previewHelp",
      "inspector.pinHelp",
    ] as const)
      expect(t(key)).not.toBe(key);
  }
});
