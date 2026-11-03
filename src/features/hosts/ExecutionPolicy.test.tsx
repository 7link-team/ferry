import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { executionText, i18n } from "../../shared/i18n/i18n";
import { defaultExecutionPolicy, ExecutionPolicyFields } from "./ExecutionPolicy";

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("keeps shared execution opt-in and blocks invalid capacity before saving", async () => {
  const validity = vi.fn();
  function Form() {
    const [value, setValue] = useState(defaultExecutionPolicy);
    return <ExecutionPolicyFields value={value} readOnly={false} onChange={setValue} onValidityChange={validity} />;
  }
  await act(async () => root.render(<Form />));
  const checkboxes = host.querySelectorAll<HTMLInputElement>('input[type="checkbox"]');
  expect(checkboxes[0]!.checked).toBe(true);
  expect(checkboxes[1]!.checked).toBe(false);
  await act(async () => checkboxes[1]!.click());
  expect(checkboxes[1]!.checked).toBe(true);
  const input = host.querySelector<HTMLInputElement>('input[type="number"]')!;
  expect(input.getAttribute("aria-label")).toBe(executionText("hostExecution.capacity"));
  for (const [value, blocked] of [["0", true], ["65", true], ["2.5", true], ["4", false]] as const) {
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(validity).toHaveBeenLastCalledWith(blocked);
  }
});

it("disables execution policy controls in the shared detail form", async () => {
  await act(async () => root.render(<ExecutionPolicyFields readOnly onChange={vi.fn()} onValidityChange={vi.fn()} />));
  expect([...host.querySelectorAll<HTMLInputElement>("input")].every(input => input.disabled)).toBe(true);
});
