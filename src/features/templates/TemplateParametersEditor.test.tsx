import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { TemplateParametersEditor, TemplateParameterPreview, templateParameterFields } from "./TemplateParametersEditor";

vi.mock("../../shared/editor/CodeEditor", () => ({ CodeEditor: ({ value, label, onChange }: { value: string; label: string; onChange: (value: string) => void }) => <textarea aria-label={label} value={value} onChange={event => onChange(event.target.value)} /> }));
let root: Root, host: HTMLDivElement, latest: string;
const valid = vi.fn();
function Harness({ source = "[]", readOnly = false }: { source?: string; readOnly?: boolean }) {
  const [value, setValue] = useState(source);
  latest = value;
  return <TemplateParametersEditor value={value} readOnly={readOnly} onChange={setValue} onValidityChange={valid} />;
}
const button = (text: string) => [...host.querySelectorAll<HTMLButtonElement>("button")].find(item => item.textContent === text)!;
async function fill(element: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, value: string) {
  await act(async () => {
    const proto = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  await i18n.changeLanguage("zh-CN");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  valid.mockClear();
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });

it("adds unique names and edits typed defaults without JSON", async () => {
  await act(async () => root.render(<Harness source='[{"name":"parameter_2","type":"string"}]' />));
  await act(async () => button("添加参数").click());
  expect(JSON.parse(latest).map((row: { name: string }) => row.name)).toEqual(["parameter_2", "parameter_3"]);
  const row = host.querySelectorAll("fieldset")[1]!;
  await fill(row.querySelector("select")!, "integer");
  await act(async () => row.querySelector<HTMLButtonElement>('[role="checkbox"]')!.click());
  await fill(row.querySelector<HTMLInputElement>('input[type="number"]')!, "42");
  expect(JSON.parse(latest)[1].default).toBe(42);
});

it("preserves existing validation rules and blocks duplicate names", async () => {
  const rows = [{ name: "image", type: "string", min_length: 2, max_length: 99, enum: ["nginx:alpine"], default: "nginx:alpine" }, { name: "port", type: "integer" }];
  await act(async () => root.render(<Harness source={JSON.stringify(rows)} />));
  await fill(host.querySelector<HTMLInputElement>("input")!, "port");
  expect(valid).toHaveBeenLastCalledWith(true);
  expect(JSON.parse(latest)[0]).toEqual({ ...rows[0], name: "port" });
  await fill(host.querySelector<HTMLInputElement>("input")!, "image_tag");
  expect(valid).toHaveBeenLastCalledWith(false);
});

it("keeps the complete definition when switching between form and source", async () => {
  const source = '[{"name":"token","type":"string","secret":true,"max_length":100}]';
  await act(async () => root.render(<Harness source={source} />));
  await act(async () => button("高级 JSON").click());
  expect(latest).toBe(source);
  await fill(host.querySelector("textarea")!, "[");
  expect(valid).toHaveBeenLastCalledWith(true);
  await fill(host.querySelector("textarea")!, source);
  expect(valid).toHaveBeenLastCalledWith(false);
  await act(async () => button("可视化表单").click());
  expect(latest).toBe(source);
});

it("shows typed values in read-only mode without mutation controls", async () => {
  await act(async () => root.render(<Harness source='[{"name":"enabled","type":"boolean","default":false}]' readOnly />));
  expect([...host.querySelectorAll("select")].every(select => select.disabled)).toBe(true);
  expect(button("添加参数")).toBeUndefined();
  expect(latest).toContain('"default":false');
});

it("previews the same typed fields used by template generation without changing definitions", async () => {
  const rows = [{ name: "image", label: "容器镜像", type: "string" as const, default: "nginx:alpine", enum: ["nginx:alpine", "nginx:stable"] }, { name: "replicas", type: "integer" as const, default: 3, min: 1, max: 10 }, { name: "enabled", type: "boolean" as const, default: false }, { name: "token", type: "string" as const, secret: true }];
  const fields = templateParameterFields(rows);
  expect(fields.map(field => field.type)).toEqual(["select", "number", "select", "text"]);
  expect(fields[3]!.label).toContain("变量");
  await act(async () => root.render(<TemplateParameterPreview value={JSON.stringify(rows)} />));
  expect(host.textContent).toContain("容器镜像");
  expect(host.querySelector<HTMLInputElement>('input[type="number"]')!.value).toBe("3");
  expect(host.querySelector<HTMLInputElement>('input[type="number"]')!.min).toBe("1");
  await fill(host.querySelector<HTMLInputElement>('input[type="number"]')!, "4");
  expect(rows[1]!.default).toBe(3);
  expect(host.textContent).toContain("预览输入不保存");
});
