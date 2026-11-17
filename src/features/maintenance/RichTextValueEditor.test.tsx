import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n, typedText } from "../../shared/i18n/i18n";
import RichTextValueEditor from "./RichTextValueEditor";

vi.mock("../../shared/editor/CodeEditor", () => ({
  CodeEditor: ({ value, label, readOnly, onChange }: { value: string; label: string; readOnly?: boolean; onChange: (value: string) => void }) =>
    <textarea aria-label={label} value={value} readOnly={readOnly} onChange={event => onChange(event.target.value)} />,
}));

let host: HTMLDivElement, root: Root;
beforeEach(async () => {
  await i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

const settle = () => act(async () => { await new Promise(resolve => setTimeout(resolve, 20)); });
const footer = '<p>© 2026 Example · <a href="https://example.invalid/privacy" target="_blank">Privacy</a></p>';

it("opens stored content without reporting a change and shows it in the visual editor", async () => {
  const onChange = vi.fn();
  await act(async () => root.render(<RichTextValueEditor value={footer} readOnly={false} onChange={onChange} />));
  await settle();
  const box = host.querySelector('[role="textbox"]')!;
  expect(box.getAttribute("aria-label")).toBe(typedText("richValue"));
  // Slate adds a zero-width spacer next to inline elements.
  expect(box.querySelector('a[href="https://example.invalid/privacy"]')?.textContent?.replace(/[\u200b\ufeff\s]+$/u, "")).toBe("Privacy");
  expect(onChange).not.toHaveBeenCalled();
});

it("keeps formatting it cannot represent by opening the HTML source with an explanation", async () => {
  const onChange = vi.fn();
  const styled = '<p><span style="font-family: Georgia;">Serif</span></p>';
  await act(async () => root.render(<RichTextValueEditor value={styled} readOnly={false} onChange={onChange} />));
  await settle();
  expect(host.querySelector('[role="alert"]')?.textContent).toContain(typedText("richtext.sourceOnly"));
  expect(host.querySelector<HTMLTextAreaElement>(`textarea[aria-label="${typedText("richtext.source")}"]`)?.value).toBe(styled);
  expect(host.querySelector('[role="textbox"][data-slate-editor]')).toBeNull();
  // Asking for the visual editor keeps the source and the explanation.
  const toggle = [...host.querySelectorAll("button")].find(button => button.textContent === typedText("richtext.visual"))!;
  await act(async () => toggle.click());
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  expect(onChange).not.toHaveBeenCalled();
});

it("inserts template variables as chips that save as exactly the same text", async () => {
  const onChange = vi.fn();
  const ref = createRef<{ insertText(value: string): void }>();
  await act(async () => root.render(<RichTextValueEditor value="" readOnly={false} onChange={onChange} insertRef={ref} />));
  await settle();
  await act(async () => ref.current!.insertText("Hello "));
  await act(async () => ref.current!.insertText("{{ user.name }}"));
  await settle();
  expect(host.querySelector('[data-placeholder-chip="{{ user.name }}"]')).not.toBeNull();
  expect(onChange).toHaveBeenLastCalledWith("<p>Hello {{ user.name }}</p>");
});

it("opens colours, sizes, rules and smaller headings in the visual editor", async () => {
  const onChange = vi.fn();
  const html = '<h4>Report</h4><p><span style="color: rgb(225, 60, 57);">failed</span> <span style="background-color: rgb(255, 236, 61); font-size: 14px;">{{ run.id }}</span></p><hr><p><img src="https://example.invalid/a.png" alt="" style="width: 300px;"></p>';
  await act(async () => root.render(<RichTextValueEditor value={html} readOnly={false} onChange={onChange} />));
  await settle();
  const box = host.querySelector('[role="textbox"]')!;
  expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(box.querySelector("h4")?.textContent).toBe("Report");
  expect(box.querySelector("hr")).not.toBeNull();
  expect(box.querySelector<HTMLImageElement>("img")?.style.width).toBe("300px");
  expect([...box.querySelectorAll<HTMLElement>("[style]")].some(element => element.style.color === "rgb(225, 60, 57)")).toBe(true);
  expect(box.querySelector('[data-placeholder-chip="{{ run.id }}"]')).not.toBeNull();
  expect(onChange).not.toHaveBeenCalled();
});

it("hides its own source switch when the host provides one", async () => {
  const labels = () => [...host.querySelectorAll("button")].map(button => button.textContent);
  await act(async () => root.render(<RichTextValueEditor value="<p>Hi</p>" readOnly={false} onChange={() => {}} sourceToggle={false} />));
  await settle();
  expect(labels()).not.toContain(typedText("richtext.source"));
  // Content only the source can keep still opens there, with a way back once it is fixed.
  await act(async () => root.render(<RichTextValueEditor key="styled" value='<p style="line-height: 2;">Hi</p>' readOnly={false} onChange={() => {}} sourceToggle={false} />));
  await settle();
  expect(host.querySelector('[role="alert"]')).not.toBeNull();
  expect(labels()).toContain(typedText("richtext.visual"));
});

it("switches to HTML source and back without losing content", async () => {
  const onChange = vi.fn();
  await act(async () => root.render(<RichTextValueEditor value="<h1>Title</h1><ul><li>One</li></ul>" readOnly={false} onChange={onChange} />));
  await settle();
  const toggle = () => [...host.querySelectorAll("button")].find(button => [typedText("richtext.source"), typedText("richtext.visual")].includes(button.textContent ?? ""))!;
  await act(async () => toggle().click());
  const source = host.querySelector<HTMLTextAreaElement>(`textarea[aria-label="${typedText("richtext.source")}"]`)!;
  expect(source.value).toBe("<h1>Title</h1><ul><li>One</li></ul>");
  await act(async () => toggle().click());
  await settle();
  expect(host.querySelector('[role="textbox"] h1')?.textContent).toBe("Title");
  expect(onChange).not.toHaveBeenCalled();
});

it("is read-only without enabled editing controls", async () => {
  const onChange = vi.fn();
  const ref = createRef<{ insertText(value: string): void }>();
  await act(async () => root.render(<RichTextValueEditor value={footer} readOnly onChange={onChange} insertRef={ref} />));
  await settle();
  const toolbar = host.querySelector('[role="toolbar"]')!;
  const editing = [...toolbar.querySelectorAll("button")].filter(button => ![typedText("richtext.source"), typedText("richtext.visual")].includes(button.textContent ?? ""));
  expect(editing.length).toBeGreaterThan(10);
  expect(editing.every(button => button.disabled)).toBe(true);
  await act(async () => ref.current!.insertText("{{x}}"));
  expect(onChange).not.toHaveBeenCalled();
  expect(host.querySelector('[role="textbox"]')?.getAttribute("contenteditable")).toBe("false");
});

it("labels every toolbar control in all six languages", async () => {
  const keys = ["richtext.undo", "richtext.redo", "richtext.blockType", "richtext.bold", "richtext.italic", "richtext.underline", "richtext.strikethrough",
    "richtext.inlineCode", "richtext.bulletedList", "richtext.numberedList", "richtext.align", "richtext.link", "richtext.image", "richtext.table",
    "richtext.fontSize", "richtext.color", "richtext.backgroundColor", "richtext.hr"] as const;
  await act(async () => root.render(<RichTextValueEditor value="" readOnly={false} onChange={() => {}} />));
  const seen = new Map<string, Set<string>>();
  for (const locale of ["en-US", "zh-CN", "zh-TW", "ja-JP", "ko-KR", "ms-MY"]) {
    await act(async () => { await i18n.changeLanguage(locale); });
    const labels = [...host.querySelectorAll('[role="toolbar"] [aria-label]')].map(element => element.getAttribute("aria-label"));
    for (const key of keys) {
      const text = typedText(key);
      expect(text, `${locale}:${key}`).not.toBe(key);
      expect(labels, `${locale}:${key}`).toContain(text);
      seen.set(key, (seen.get(key) ?? new Set()).add(text));
    }
  }
  // English and Chinese labels differ for every control (no untranslated fallbacks).
  for (const key of keys) expect(seen.get(key)!.size, key).toBeGreaterThan(1);
});
