import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { initialValue, parseJson, sanitizeRichText, TypedSchemaEditor, TypedValueEditor, typedValueError, VALUE_TYPES, type TypedValueRow } from "./TypedValueEditor";
import { typedResourceForm, typedResourceList } from "./typed-resources";
import { i18n, typedText } from "../i18n";
import typedMessages from "../locales/typed-domain.json";

let root: Root, host: HTMLDivElement;
const blocked = vi.fn();
let saved: Partial<TypedValueRow>;
function Harness({ initial, schema = false }: { initial: Partial<TypedValueRow>; schema?: boolean }) {
  const [draft, setDraft] = useState(initial);
  saved = draft;
  const props = { workspace: "ops", resource: "dictionaries" as const, draft, readOnly: false, onPatch: (patch: Partial<TypedValueRow>) => setDraft(current => ({ ...current, ...patch })), onValidityChange: blocked };
  return schema ? <TypedSchemaEditor {...props} /> : <TypedValueEditor {...props} />;
}
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  blocked.mockClear(); host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
const click = async (label: string) => { const button = [...host.querySelectorAll("button")].find(element => element.textContent === label || element.getAttribute("aria-label") === label)!; expect(button).toBeDefined(); await act(async () => button.click()); };
const fill = async (label: string, value: string) => { const input = host.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!; expect(input).not.toBeNull(); await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value); input.dispatchEvent(new Event("input", { bubbles: true })); }); };

it("supports every reference value type with valid initial serialized data", () => {
  expect(VALUE_TYPES).toHaveLength(12);
  for (const [type] of VALUE_TYPES) expect(typedValueError(type, initialValue(type), {}), type).toBe("");
});
it("provides every typed-editor message and interpolation parameter in all six languages", async () => {
  const locales = ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"];
  for (const [key, values] of Object.entries(typedMessages)) {
    expect(values, key).toHaveLength(locales.length);
    const params = (value: string) => [...value.matchAll(/{{(\w+)}}/g)].map(match => match[1]).sort();
    for (const value of values) {
      expect(value.trim(), key).not.toBe("");
      expect(params(value), key).toEqual(params(values[0]!));
    }
  }
  for (const locale of locales) {
    await i18n.changeLanguage(locale);
    expect(typedValueError("num", "-1", { min: 0 })).toBe(typedText("minimum", { value: 0 }));
    expect(typedText("optionLabel", { index: 3 })).not.toMatch(/{{|typed-domain/);
  }
});
it("changes option labels and validation language without resetting draft values or order", async () => {
  await act(async () => root.render(<Harness initial={{ value_type: "selection", value_json: '[{"value":"a","label":"My own label"}]' }} />));
  await fill("选项 1 显示名称", "User draft");
  await click("添加选项");
  const draft = saved.value_json;
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("请填写");
  await act(async () => { await i18n.changeLanguage("en-US"); });
  expect(saved.value_json).toBe(draft);
  expect(host.querySelector<HTMLInputElement>('[aria-label="Option 1 display name"]')?.value).toBe("User draft");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("Enter a value and display name for every option.");
  await act(async () => { await i18n.changeLanguage("ja-JP"); });
  expect(saved.value_json).toBe(draft);
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("各選択肢の値と表示名を入力してください。");
  expect(blocked).toHaveBeenLastCalledWith(true);
});
it("validates real zero bounds, decimals, invalid JSON and Unicode character counts", () => {
  expect(typedValueError("num", "-1", { min: 0 })).toContain("不能小于 0");
  expect(typedValueError("num", "1", { max: 0 })).toContain("不能大于 0");
  expect(typedValueError("num", "0.25", { decimals: 1 })).toContain("1 位小数");
  expect(typedValueError("num", "0.2", { decimals: 1 })).toBe("");
  expect(typedValueError("json", "{bad", {})).toContain("JSON");
  expect(typedValueError("text", JSON.stringify("😀"), { maxLength: 1 })).toBe("");
});
it("rejects duplicate or empty options, contradictory limits, oversize media", () => {
  expect(typedValueError("selection", '[{"value":"a","label":"A"},{"key":"a","label":"B"}]', {})).toContain("重复");
  expect(typedValueError("selection", '[{"value":"","label":"A"}]', {})).toContain("填写");
  expect(typedValueError("selection", "[]", { minSelect: 3, maxSelect: 1 })).toContain("最少选择数");
  expect(typedValueError("image", '[{"size":2048}]', { maxSizeMb: 0.001 })).toContain("最大");
});
it("edits and reorders options while keeping legacy items synchronized", async () => {
  await act(async () => root.render(<Harness initial={{ value_type: "selection", value_json: '[{"value":"a","label":"Alpha"},{"value":"b","label":"Beta"}]' }} />));
  await fill("选项 1 显示名称", "Updated");
  expect(parseJson<Record<string, string>[]>(saved.value_json, [])[0]!.label).toBe("Updated");
  expect(saved.items).toBe(saved.value_json);
  await click("下移选项 1");
  expect(parseJson<Record<string, string>[]>(saved.value_json, [])[0]!.value).toBe("b");
  await click("添加选项");
  expect(blocked).toHaveBeenLastCalledWith(true);
  await fill("选项 3 值", "c"); await fill("选项 3 显示名称", "Third");
  expect(blocked).toHaveBeenLastCalledWith(false);
  await click("删除选项 3"); expect(parseJson<unknown[]>(saved.value_json, [])).toHaveLength(2);
});
it("locks host-state keys and deletions but permits label editing and ordering", async () => {
  await act(async () => root.render(<Harness initial={{ key: "host-state", builtin: true, value_type: "selection", value_json: '[{"value":"online","label":"Online"},{"value":"offline","label":"Offline"}]' }} />));
  expect(host.querySelector<HTMLInputElement>('[aria-label="选项 1 值"]')!.disabled).toBe(true);
  expect(host.querySelector<HTMLButtonElement>('[aria-label="删除选项 1"]')!.disabled).toBe(true);
  expect(host.textContent).not.toContain("添加选项");
  await fill("选项 1 显示名称", "Connected"); await click("下移选项 1");
  expect(parseJson<Record<string, string>[]>(saved.value_json, [])[1]!.label).toBe("Connected");
});
it("preserves an explicit zero constraint and can remove it", async () => {
  await act(async () => root.render(<Harness schema initial={{ value_type: "num", schema_json: "{}" }} />));
  await fill("最小值", "0"); expect(JSON.parse(saved.schema_json!)).toEqual({ min: 0 });
  await fill("最小值", ""); expect(JSON.parse(saved.schema_json!)).toEqual({});
});
it("isolates active HTML previews and sanitizes rich text before insertion", async () => {
  const unsafe = '<p onclick="alert(1)">Hello</p><img src=x onerror="alert(2)"><script>alert(3)</script><b>bold</b>';
  const cleaned = sanitizeRichText(unsafe);
  expect(cleaned).not.toMatch(/onclick|onerror|<script/); expect(cleaned).toContain("<b>bold</b>");
  expect(sanitizeRichText('<a href="https://example.invalid">Link</a><table><tbody><tr><td>Cell</td></tr></tbody></table>')).toContain('<a href="https://example.invalid">Link</a>');
  expect(sanitizeRichText('<a href="javascript:alert(1)">Link</a>')).not.toContain("javascript:");
  await act(async () => root.render(<Harness initial={{ value_type: "html", value_json: JSON.stringify(unsafe) }} />));
  const preview = host.querySelector("iframe")!;
  expect(preview.getAttribute("sandbox")).toBe("");
  expect(preview.getAttribute("srcdoc")).toContain("default-src 'none'");
});
it("renders Markdown tables and code while keeping raw HTML inactive", async () => {
  const text = "# Heading\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```js\nlet a = 1;\n```\n\n<script>alert(1)</script>";
  await act(async () => root.render(<Harness initial={{ value_type: "markdown", value_json: JSON.stringify(text) }} />));
  await act(async () => { await vi.dynamicImportSettled(); });
  expect(host.querySelector("h1")?.textContent).toBe("Heading");
  expect(host.querySelector("td")?.textContent).toBe("1");
  expect(host.querySelector("pre code")?.textContent).toContain("let a = 1");
  expect(host.querySelector("script")).toBeNull();
});
it("keeps built-in identities protected and enables typed custom configuration creation", () => {
  const config = typedResourceForm("settings");
  expect(config.blank.value_type).toBe("text");
  const fields = config.sections.flatMap(section => section.fields);
  expect(fields.find(field => field.key === "key")!.immutable).toBe(true);
  expect(fields.find(field => field.key === "value_type")!.disabled!({ builtin: true })).toBe(true);
  const list = typedResourceList("ops", "settings");
  expect(list.toolbar?.create).not.toBe(false);
  expect(list.actions!.find(action => action.label === "删除")!.when!({ builtin: true } as TypedValueRow)).toBe(false);
});
