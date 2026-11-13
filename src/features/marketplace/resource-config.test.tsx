import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { hostRepositoryText, i18n } from "../../shared/i18n/i18n";
import { marketplaceForms } from "./resource-config";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, readOnly, label }: { value: string; readOnly: boolean; label: string }) => <textarea aria-label={label} value={value} readOnly={readOnly} /> }));
let root: Root, host: HTMLDivElement;
const patch = vi.fn(), validity = vi.fn();
const manifest = JSON.stringify({ apiVersion: "ferry/v1", kind: "Application", metadata: { name: "sample-http" }, services: [{ name: "web", image: "busybox:1.36", ports: [{ container: 8080, published: "{{ .Values.port }}" }] }] }, null, 2);
const config = () => marketplaceForms({ workspace: "test", navigate: vi.fn() } as never).marketplace!;
async function show(key: string, draft: Record<string, unknown>) {
  const field = config().sections.flatMap(section => section.fields).find(field => field.key === key)!;
  await act(async () => root.render(<>{field.render!({ workspace: "test", draft, readOnly: true, onPatch: patch, onValidityChange: validity })}</>));
}
beforeEach(async () => {
  await i18n.changeLanguage("en-US");
  host = document.createElement("div");document.body.append(host);root = createRoot(host);
  patch.mockClear();validity.mockClear();
});
afterEach(async () => { await act(async () => root.unmount());host.remove(); });

it("renders application services visually by default and keeps the original source readonly", async () => {
  await show("content", { content: manifest });
  expect([...host.querySelectorAll("textarea")].some(input => input.value === manifest)).toBe(false);
  expect(host.textContent).toContain("web");
  expect([...host.querySelectorAll<HTMLInputElement>("input")].some(input => input.value === "busybox:1.36")).toBe(true);
  const source = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === hostRepositoryText("manifest.019"))!;
  await act(async () => source.click());
  expect(host.querySelector("textarea")!.value).toBe(manifest);
  expect(host.querySelector("textarea")!.readOnly).toBe(true);
  expect(patch).not.toHaveBeenCalled();
});

it("renders parameter definitions with false and zero defaults instead of raw JSON", async () => {
  const parameters = JSON.stringify([{ name: "enabled", label: "Enable example", type: "boolean", default: false }, { name: "count", type: "integer", default: 0 }]);
  await show("parameters", { parameters });
  expect(host.querySelector("dl")?.textContent).toContain("No");
  expect(host.textContent).toContain("Enable example");
  expect(host.textContent).toContain("count");
  expect(host.querySelector("input, select, textarea, [role=switch]")).toBeNull();
  const advanced = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => /JSON/.test(button.textContent ?? ""))!;
  await act(async () => advanced.click());
  expect(host.querySelector("textarea")!.value).toBe(parameters);
  expect(host.querySelector("textarea")!.readOnly).toBe(true);
  expect(patch).not.toHaveBeenCalled();
});

it("renders README headings and lists safely with a translated accessible label", async () => {
  await show("readme", { readme: "# Sample documentation\n\n- Port 8080\n\n<script>alert('untrusted')</script>" });
  // The markdown module is lazy-loaded; allow its import promise to settle.
  await vi.waitFor(() => expect(host.querySelector("h1")?.textContent).toBe("Sample documentation"));
  expect(host.querySelector("li")?.textContent).toBe("Port 8080");
  expect(host.querySelector("script")).toBeNull();
  expect(host.querySelector('[aria-label="Documentation"]')).toBeTruthy();
  expect(patch).not.toHaveBeenCalled();
});

it("preserves malformed source for inspection instead of pretending it is an empty template", async () => {
  await show("content", { content: "not: [valid" });
  expect(host.querySelector("textarea")!.value).toBe("not: [valid");
  expect(host.querySelector("textarea")!.readOnly).toBe(true);
  expect(host.querySelector('[role="alert"]')).toBeTruthy();
  expect(patch).not.toHaveBeenCalled();
});
