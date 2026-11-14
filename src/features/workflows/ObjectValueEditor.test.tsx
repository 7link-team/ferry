import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { ObjectValueEditor } from "./WorkflowNodeParameters";

vi.mock("../../shared/editor/CodeEditor", () => ({
  CodeEditor: ({ value, label, readOnly, onChange }: { value: string; label: string; readOnly?: boolean; onChange: (value: string) => void }) =>
    <textarea aria-label={label} value={value} readOnly={readOnly} onChange={event => onChange(event.target.value)} />,
}));

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

const big = { contributors: Array.from({ length: 200 }, (_, i) => ({ name: `user-${i}`, commits: i, additions: i * 10 })) };
const type = async (area: HTMLTextAreaElement, text: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(area, text);
  area.dispatchEvent(new Event("input", { bubbles: true }));
});
const yamlArea = async () => {
  await vi.waitFor(() => expect(host.querySelector("textarea")).toBeTruthy());
  return host.querySelector("textarea")!;
};

it("opens large values as YAML, applies valid edits and rejects invalid ones without changing the value", async () => {
  const change = vi.fn();
  await act(async () => root.render(<ObjectValueEditor label="值" value={big} array={false} readOnly={false} change={change} />));
  const area = await yamlArea();
  expect(area.value).toContain("contributors:");
  expect(host.querySelector('[aria-pressed="true"]')?.textContent).toBe("YAML");

  await type(area, "a: 1\nb:\n  - x\n");
  expect(change).toHaveBeenLastCalledWith({ a: 1, b: ["x"] });

  change.mockClear();
  await type(area, "a: [unclosed");
  expect(change).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("YAML");

  await type(area, "- just\n- a list\n");
  expect(change).not.toHaveBeenCalled();
  expect(host.querySelector('[role="alert"]')).toBeTruthy();
});

it("keeps small values in the form and lets the user switch to YAML", async () => {
  const change = vi.fn();
  await act(async () => root.render(<ObjectValueEditor label="值" value={{ a: "1" }} array={false} readOnly={false} change={change} />));
  expect(host.querySelector("textarea")).toBeNull();
  const yaml = [...host.querySelectorAll("button")].find(button => button.textContent === "YAML")!;
  await act(async () => yaml.click());
  expect((await yamlArea()).value.trim()).toBe('a: "1"');
});

it("shows read-only values without a mode switch", async () => {
  await act(async () => root.render(<ObjectValueEditor label="值" value={big} array={false} readOnly change={() => {}} />));
  const area = await yamlArea();
  expect(area.readOnly).toBe(true);
  expect(host.querySelector('[role="group"]')).toBeNull();
  expect([...host.querySelectorAll("button")].every(button => button.disabled)).toBe(true);
});
