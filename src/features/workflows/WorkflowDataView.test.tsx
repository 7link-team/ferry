import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it } from "vitest";
import { WorkflowDataView } from "./WorkflowDataView";

let root: Root, host: HTMLDivElement;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

it("shows an accessible skeleton only while waiting for the first result", async () => {
  await act(async () => root.render(<WorkflowDataView items={[]} loading empty="No result" />));
  expect(host.querySelector('[data-slot="loading-skeleton"]')?.getAttribute("aria-busy")).toBe("true");
  expect(host.textContent).not.toContain("No result");
  await act(async () => root.render(<WorkflowDataView items={[]} empty="No result" />));
  expect(host.querySelector('[data-slot="loading-skeleton"]')).toBeNull();
  expect(host.textContent).toContain("No result");
});

it("keeps existing output and the selected view during a refresh", async () => {
  const items = [{ json: { message: "Existing output" } }];
  await act(async () => root.render(<WorkflowDataView items={items} />));
  const buttons = host.querySelectorAll<HTMLButtonElement>('[role="group"] button');
  await act(async () => buttons[2]!.click());
  await act(async () => root.render(<WorkflowDataView items={items} loading />));
  expect(host.querySelector('[data-slot="loading-skeleton"]')).toBeNull();
  expect(host.querySelector("pre")?.textContent).toContain("Existing output");
  expect(buttons[2]!.getAttribute("aria-pressed")).toBe("true");
});
