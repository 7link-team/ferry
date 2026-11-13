import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ScrollFormSections } from "../../shared/ui/scroll-form-sections";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("matchMedia", () => ({ matches: true }));
  history.replaceState({}, "", "/zh-CN/workspaces/test/app-templates/new");
  host = document.createElement("div"); host.style.overflowY = "auto"; document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("navigates one continuous form without unmounting fields or losing unsaved input", async () => {
  await act(async () => root.render(<ScrollFormSections name="Current template" sections={[
    { key: "basics", label: "Basics", content: <input defaultValue="" /> },
    { key: "definition", label: "Definition", content: <textarea defaultValue="services: []" /> },
    { key: "parameters", label: "Parameters", content: <p>Parameter definitions</p> },
  ]} footer={<button>Save</button>} />));
  expect(host.querySelectorAll("[data-form-section]")).toHaveLength(3);
  const input = host.querySelector("input")!; input.value = "Unsaved template";
  await act(async () => host.querySelector<HTMLAnchorElement>('a[aria-controls$="-parameters"]')!.click());
  expect(host.querySelector("input")).toBe(input);
  expect(input.value).toBe("Unsaved template");
  expect(host.querySelector("textarea")!.value).toBe("services: []");
  expect(new URLSearchParams(location.search).get("section")).toBe("parameters");
  expect(host.querySelector("h2")!.textContent).toBe("Current template");
  expect(host.querySelector('a[aria-current="location"]')!.textContent).toBe("Parameters");
});
