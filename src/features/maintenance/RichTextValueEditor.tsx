/** Source wangEditor integration using its native lifecycle, independent of React wrapper versions. */
import { createEditor,createToolbar,i18nAddResources,i18nChangeLanguage,type IDomEditor } from "@wangeditor-next/editor";
import "@wangeditor-next/editor/dist/css/style.css";
import { useEffect,useRef,type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { typedText } from "../../shared/i18n/i18n";
import richTextMessages from "../../shared/i18n/locales/richtext-domain.json";
import { sanitizeRichText } from "./value-content";

// The editor supplies English and Simplified Chinese. Its native language event
// rebuilds menus while retaining the editor's document and undo history.
for (const [column, locale] of ["zh-TW", "ja-JP", "ko-KR", "ms-MY"].entries()) {
  const resources: Record<string, unknown> = {};
  for (const [path, values] of Object.entries(richTextMessages)) {
    const parts = path.split(".");
    let group = resources;
    for (const part of parts.slice(0, -1)) {
      group[part] ??= {};
      group = group[part] as Record<string, unknown>;
    }
    group[parts.at(-1)!] = values[column];
  }
  i18nAddResources(locale, resources);
}

export default function RichTextValueEditor({ value, readOnly, onChange }: { value: string; readOnly: boolean; onChange: (value: string) => void }) {
  const { i18n } = useTranslation("typed-domain");
  const toolbarContainer = useRef<HTMLDivElement>(null);
  const editorContainer = useRef<HTMLDivElement>(null);
  const editorRef = useRef<IDomEditor | null>(null);
  const changeRef = useRef(onChange);
  changeRef.current = onChange;
  const lastHtml = useRef(value);
  useEffect(() => {
    i18nChangeLanguage(i18n.language === "en-US" ? "en" : i18n.language);
  }, [i18n.language]);
  useEffect(() => {
    if (!editorContainer.current || !toolbarContainer.current) return;
    const editor = createEditor({
      selector: editorContainer.current,
      html: sanitizeRichText(value),
      mode: "default",
      config: { readOnly, onChange: current => { const html = sanitizeRichText(current.getHtml()); lastHtml.current = html; changeRef.current(html); } },
    });
    editorRef.current = editor;
    createToolbar({ editor, selector: toolbarContainer.current, mode: "simple" });
    const labelEditable = () => {
      const textbox = editorContainer.current?.querySelector('[data-slate-editor]');
      if (!textbox) return;
      textbox.setAttribute("role", "textbox");
      textbox.setAttribute("aria-label", typedText("richValue"));
      textbox.setAttribute("aria-multiline", "true");
      observer.disconnect();
    };
    const observer = new MutationObserver(labelEditable);
    observer.observe(editorContainer.current, { childList: true, subtree: true });
    labelEditable();
    return () => { observer.disconnect(); editor.destroy(); editorRef.current = null; };
    // Source keeps exactly one instance for the mounted editor; separate effects update value/mode.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    editorContainer.current?.querySelector('[data-slate-editor]')?.setAttribute("aria-label", typedText("richValue"));
  }, [i18n.language]);
  useEffect(() => { const editor = editorRef.current; if (editor) { if (readOnly) editor.disable(); else editor.enable(); } }, [readOnly]);
  useEffect(() => { const editor = editorRef.current; if (editor && value !== lastHtml.current) { lastHtml.current = value; editor.setHtml(sanitizeRichText(value)); } }, [value]);
  return <div className="overflow-hidden rounded-md border" style={{
    "--w-e-textarea-bg-color": "var(--background)", "--w-e-textarea-color": "var(--foreground)",
    "--w-e-toolbar-bg-color": "var(--card)", "--w-e-toolbar-color": "var(--foreground)",
    "--w-e-toolbar-active-bg-color": "var(--accent)", "--w-e-toolbar-active-color": "var(--accent-foreground)",
    "--w-e-toolbar-disabled-color": "var(--muted-foreground)", "--w-e-toolbar-border-color": "var(--border)",
    "--w-e-modal-button-bg-color": "var(--secondary)", "--w-e-modal-button-color": "var(--secondary-foreground)",
  } as CSSProperties}>
    <div ref={toolbarContainer} className="border-b" />
    <div ref={editorContainer} className="h-60 overflow-y-hidden" />
  </div>;
}
