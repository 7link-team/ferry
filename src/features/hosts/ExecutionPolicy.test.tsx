import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { commonText, executionText, i18n } from "../../shared/i18n/i18n";
import { defaultExecutionPolicy, ExecutionPolicyFields } from "./ExecutionPolicy";

let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("keeps execution access controls and leaves resource ceilings to platform settings", async () => {
  const validity = vi.fn();
  function Form() {
    const [value, setValue] = useState(defaultExecutionPolicy);
    return <ExecutionPolicyFields value={value} readOnly={false} onChange={setValue} onValidityChange={validity} />;
  }
  await act(async () => root.render(<Form />));
  const checkboxes = host.querySelectorAll<HTMLButtonElement>('[role="switch"]');
  expect(checkboxes[0]!.getAttribute("aria-checked")).toBe("true");
  expect(checkboxes[1]!.getAttribute("aria-checked")).toBe("false");
  await act(async () => checkboxes[1]!.click());
  expect(checkboxes[1]!.getAttribute("aria-checked")).toBe("true");
  expect(host.querySelector('input[type="number"]')).toBeNull();
  expect(validity).toHaveBeenLastCalledWith(false);

});

it("renders execution policy as text in the shared detail form", async () => {
  const onChange = vi.fn();
  const validity = vi.fn();
  await act(async () => root.render(<ExecutionPolicyFields value={{ enabled: true, shared: false, capacity: 4 }} readOnly onChange={onChange} onValidityChange={validity} />));
  expect(host.querySelectorAll("input, select, textarea")).toHaveLength(0);
  expect([...host.querySelectorAll("dt")].map(node => node.textContent)).toEqual([
    executionText("hostExecution.enabled"), executionText("hostExecution.shared"),
  ]);
  expect([...host.querySelectorAll("dd")].map(node => node.textContent)).toEqual([
    commonText("common.yes"), commonText("common.no"),
  ]);
  expect(onChange).not.toHaveBeenCalled();
  expect(validity).toHaveBeenLastCalledWith(false);
});
