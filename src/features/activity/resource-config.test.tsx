import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { i18n } from "../../shared/i18n/i18n";
import { activityList } from "./resource-config";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, readOnly, label }: { value: string; readOnly: boolean; label: string }) => <textarea aria-label={label} readOnly={readOnly} value={value} /> }));
let root: Root, host: HTMLDivElement;
const row = { id: "9", actor_name: "Operator", action: "members", resource: "conversations", resource_id: "123", resource_name: "Current room", details: {}, created_at: 1794378000000 };
const copy = vi.fn();
const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard");
beforeEach(async () => {
  await i18n.changeLanguage("en-US");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  copy.mockReset().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: copy } });
});
afterEach(async () => {
  await act(async () => root.unmount());host.remove();
  if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard);
  else Reflect.deleteProperty(navigator, "clipboard");
});
async function open() {
  const render = activityList().columns.find(column => column.key === "details")!.render!;
  await act(async () => root.render(<>{render(row)}</>));
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
  return document.querySelector('[role="dialog"]') as HTMLElement;
}

it("opens a read-only audit panel with context, explicit missing details and fixed chrome", async () => {
  const dialog = await open();
  expect(dialog.textContent).toContain("Activity details");
  expect(dialog.textContent).toContain("Adjusted conversation members");
  expect(dialog.textContent).toContain("Current room");
  expect(dialog.textContent).toContain("Operator");
  expect(dialog.textContent).toContain("Change details were not recorded.");
  expect(dialog.querySelector("header")).toBeTruthy();
  expect(dialog.querySelector("footer.dialog-footer")).toBeTruthy();
  expect(dialog.querySelector('[role="tabpanel"]')!.classList.contains("dialog-body")).toBe(true);
  await act(async () => dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click());
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(document.activeElement).toBe(host.querySelector("button"));
});

it("shows formatted JSON in the shared read-only editor and copies the original record", async () => {
  const dialog = await open();
  const tab = [...dialog.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(el => el.textContent === "Raw JSON")!;
  await act(async () => { tab.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })); });
  const editor = dialog.querySelector<HTMLTextAreaElement>('textarea[aria-label="Raw JSON"]')!;
  expect(editor.readOnly).toBe(true);
  expect(editor.value).toBe(JSON.stringify(row, null, 2));
  const button = [...dialog.querySelectorAll<HTMLButtonElement>("button")].find(el => el.textContent === "Copy JSON")!;
  await act(async () => button.click());
  expect(copy).toHaveBeenCalledWith(JSON.stringify(row, null, 2));
  expect(dialog.querySelector('[role="status"]')?.textContent).toContain("Copied");
});

it("reports copy failure and keeps the panel usable", async () => {
  copy.mockRejectedValue(new Error("Clipboard denied"));
  const dialog = await open();
  await act(async () => [...dialog.querySelectorAll<HTMLButtonElement>("button")].find(el => el.textContent === "Copy JSON")!.click());
  expect(dialog.querySelector('[role="alert"]')?.textContent).toContain("Copy failed");
  expect(document.querySelector('[role="dialog"]')).toBe(dialog);
});

it("does not hide stored execution IDs, permissions, nested monitoring, false or zero", () => {
  const render = activityList().columns.find(column => column.key === "details")!.render!;
  const html = renderToStaticMarkup(<>{render({ ...row, action: "update", resource: "workflows", details: {
    execution_id: "exec-123", revision_id: "rev-456", permissions: ["hosts.read", "workflows.run"],
    monitoring: { enabled: false, interval: 0 }, secret_rotated: false, target_count: 0,
    unknown_future_field: "kept-visible", name: "Only saved name",
  } })}</>);
  for (const value of ["exec-123", "rev-456", "hosts.read", "workflows.run", "interval", "false", "kept-visible", "Only saved name", ">0<", ">No<"])
    expect(html).toContain(value);
  expect(html).toContain("previous values were not saved");
});

it("shows actual before and after values, including clearing, missing fields and unchanged submissions", () => {
  const render = activityList().columns.find(column => column.key === "details")!.render!;
  const html = renderToStaticMarkup(<>{render({ ...row, action: "update", details: { changes: {
    title: { before: "Old room", after: "New room" },
    enabled: { before: true, after: false },
    color: { before: "blue", after: "" },
    count: { before: 1, after: 0 },
    removed: { before: null, after: null, before_present: true, after_present: false },
  } } })}</>);
  for (const text of ["Before", "After", "Old room", "New room", "Empty text", "Field absent", ">No<", ">0<"])
    expect(html).toContain(text);
  expect(html).not.toContain("previous values were not saved");
  const unchanged = renderToStaticMarkup(<>{render({ ...row, action: "update", details: { changes: {} } })}</>);
  expect(unchanged).toContain("Submitted values are unchanged.");
  const sameMembers = renderToStaticMarkup(<>{render({ ...row, details: { name: "Same room", membership: {
    members: { added: [], removed: [] }, agents: { added: [], removed: [] },
  } } })}</>);
  expect(sameMembers).toContain("Member and Agent lists unchanged.");
});

it("keeps name-only records visible and bounds large previews without losing raw data", () => {
  const render = activityList().columns.find(column => column.key === "details")!.render!;
  expect(renderToStaticMarkup(<>{render({ ...row, action: "update", details: { name: "Saved name only" } })}</>))
    .toContain("Saved name only");
  const html = renderToStaticMarkup(<>{render({ ...row, details: { long: "x".repeat(10000) + "end-of-value" } })}</>);
  expect(html).toContain("See Raw JSON for the full content.");
  expect(html).not.toContain("end-of-value");
});
