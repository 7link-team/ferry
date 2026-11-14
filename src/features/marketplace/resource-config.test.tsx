import { parse } from "yaml";
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
  expect(host.querySelector('[data-application-definition]')).not.toBeNull();
  expect(host.querySelector('[data-blueprint-group="services"]')?.textContent).toContain("busybox:1.36");
  expect(host.querySelector('input,select,textarea')).toBeNull();
  const source = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === hostRepositoryText("manifest.sourcePreview"))!;
  await act(async () => source.click());
  expect(parse(host.querySelector("textarea")!.value)).toEqual(JSON.parse(manifest));
  expect(host.querySelector("textarea")!.readOnly).toBe(true);
  expect(patch).not.toHaveBeenCalled();
});

it("shows included file contents, mount paths and parameters after the deployment definition without editing", async () => {
  const fields = config().sections.flatMap(section => section.fields);
  expect(fields.findIndex(field => field.key === "content")).toBeLessThan(fields.findIndex(field => field.key === "files"));
  const content = JSON.stringify({kind:"Blueprint",roles:[{service:{files:[{path:"/etc/nats/nats.conf",fromTemplate:"nats.conf"}]}}]});
  await show("files", { id:"market-1", content, files:["nats.conf"], package_files:[{name:"nats.conf",content:"server_name: {{ .Instance.Name }}",parameters:[{name:"port",type:"integer",default:4222}]}] });
  expect(host.querySelector("summary")?.textContent).toBe("nats.conf");
  expect(host.textContent).toContain("/etc/nats/nats.conf");
  expect(host.querySelector('textarea[aria-label="nats.conf"]')?.getAttribute("readonly")).not.toBeNull();
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="nats.conf"]')?.value).toBe("server_name: {{ .Instance.Name }}");
  expect(host.querySelector('button')).toBeNull();
  expect(host.querySelector('input,select')).toBeNull();
  expect(patch).not.toHaveBeenCalled();
});

it("reports unavailable included content and an absent reference without inventing configuration", async () => {
  await show("files", {content:"not: [valid",files:["missing.conf"]});
  expect(host.textContent).toContain("File content is unavailable in this response.");
  expect(host.textContent).toContain("No direct reference found in this definition");
  expect(host.querySelector("textarea")).toBeNull();
});

it("renders parameter definitions with false and zero defaults instead of raw JSON", async () => {
  const parameters = JSON.stringify([{ name: "enabled", label: "Enable example", type: "boolean", default: false }, { name: "count", type: "integer", default: 0 }]);
  await show("parameters", { parameters });
  expect([...host.querySelectorAll<HTMLSelectElement>('select')].some(control=>control.value==='false' && control.disabled)).toBe(true);
  expect(host.textContent).toContain("Enable example");
  expect(host.textContent).toContain("count");
  expect(host.querySelector('section[aria-label="Usage form preview"]')).toBeNull();
  const preview = [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === 'Usage form preview')!;
  await act(async () => preview.click());
  expect(host.querySelector<HTMLInputElement>('input[type="number"]')!.value).toBe('0');
  await act(async () => preview.click());
  expect([...host.querySelectorAll<HTMLInputElement>('input, select, textarea, [role=switch]')].every(control=>control.disabled)).toBe(true);
  const advanced = [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => /JSON/.test(button.textContent ?? ""))!;
  await act(async () => advanced.click());
  expect(host.querySelector("textarea")!.value).toBe(parameters);
  expect(host.querySelector("textarea")!.readOnly).toBe(true);
  expect(patch).not.toHaveBeenCalled();
});

it("renders cluster blueprints as readonly fields with one functioning source/form toggle", async () => {
  const blueprint = { apiVersion: "ferry/blueprint-v1", kind: "Blueprint", metadata: { name: "nats-cluster" }, allowSingleHost: false,
    roles: [{ name: "nats", instances: { default: 3, min: 3, max: 9, odd: true }, service: { image: "nats:2.10-alpine" }, ports: [{ published: "{{ .Values.client_port }}", container: 4222 }] }], endpoints: [] };
  await show("content", { content: JSON.stringify(blueprint), parameters: "[]" });
  expect(host.querySelector('[data-blueprint-form]')).not.toBeNull();
  expect(host.querySelector('[role="alert"]')).toBeNull();
  for (const value of ['nats-cluster', 'nats:2.10-alpine', 'No', '3', '[]', '{{ .Values.client_port }}']) expect(host.textContent).toContain(value);
  expect(host.querySelector('input, select, textarea')).toBeNull();
  expect(host.querySelector('[data-blueprint-group="roles"]')?.textContent).toContain('nats:2.10-alpine');
  expect(host.querySelectorAll('button')).toHaveLength(1);
  await act(async () => host.querySelector('button')!.click());
  expect(host.querySelector('textarea')!.readOnly).toBe(true);
  expect(host.querySelector('textarea')!.value).toContain('apiVersion: ferry/blueprint-v1');
  await act(async () => host.querySelector('button')!.click());
  expect(host.querySelector('[data-blueprint-form]')).not.toBeNull();
  expect(patch).not.toHaveBeenCalled();
});

it("renders README headings and lists safely with a translated accessible label", async () => {
  await show("readme", { readme: "# Sample documentation\n\n- Port 8080\n\n<script>alert('untrusted')</script>" });
  // The markdown module is lazy-loaded; allow its import promise to settle.
  // The Markdown renderer loads lazily; allow for a loaded test machine.
  await vi.waitFor(() => expect(host.querySelector("h1")?.textContent).toBe("Sample documentation"), { timeout: 5000 });
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
