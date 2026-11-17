/**
 * Rich-text editing for stored HTML (mail templates, footer, settings,
 * notification bodies) on Plate. HTML is converted by `rich-text/html.ts`;
 * content the visual editor cannot keep exactly opens as HTML source instead
 * of being changed silently.
 */
import { BlockquotePlugin, BoldPlugin, CodePlugin, H1Plugin, H2Plugin, H3Plugin, ItalicPlugin, StrikethroughPlugin, UnderlinePlugin } from "@platejs/basic-nodes/react";
import { setAlign } from "@platejs/basic-styles";
import { TextAlignPlugin } from "@platejs/basic-styles/react";
import { toggleCodeBlock } from "@platejs/code-block";
import { CodeBlockPlugin } from "@platejs/code-block/react";
import { unwrapLink, upsertLink } from "@platejs/link";
import { LinkPlugin } from "@platejs/link/react";
import { toggleList } from "@platejs/list-classic";
import { ListPlugin } from "@platejs/list-classic/react";
import { ImagePlugin } from "@platejs/media/react";
import { insertTable } from "@platejs/table";
import { TablePlugin } from "@platejs/table/react";
import { AlignCenter, AlignJustify, AlignLeft, AlignRight, Bold, ChevronDown, Code, CodeXml, Image as ImageIcon, Italic, Link2, List, ListOrdered, Redo2, Strikethrough, Table, Underline, Undo2 } from "lucide-react";
import { KEYS, type SlateEditor, type TElement, type Value } from "platejs";
import { createPlatePlugin, Plate, PlateContent, useEditorSelector, usePlateEditor, type PlateEditor } from "platejs/react";
import { Popover } from "radix-ui";
import { useEffect, useImperativeHandle, useRef, useState, type ReactNode, type Ref } from "react";
import { useTranslation } from "react-i18next";
import { CodeEditor } from "../../shared/editor/CodeEditor";
import { AutoForm } from "../../shared/forms/auto-form";
import { typedText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../../shared/ui/dropdown-menu";
import { InlineAlert } from "../../shared/ui/inline-alert";
import { Input } from "../../shared/ui/input";
import { components } from "./rich-text/elements";
import { ALIGNMENTS, htmlToValue, isPlaceholder, MARKS, PLACEHOLDER, PLACEHOLDER_PATTERN, roundTrips, valueToHtml, type Mark } from "./rich-text/html";
import { sanitizeRichText } from "./value-content";

const PlaceholderPlugin = createPlatePlugin({ key: PLACEHOLDER, node: { isElement: true, isInline: true, isVoid: true } });
const plugins = [
  H1Plugin, H2Plugin, H3Plugin, BlockquotePlugin,
  BoldPlugin, ItalicPlugin, UnderlinePlugin, StrikethroughPlugin, CodePlugin,
  TextAlignPlugin.configure({ inject: { targetPlugins: [KEYS.p, KEYS.h1, KEYS.h2, KEYS.h3, KEYS.blockquote] } }),
  ListPlugin, LinkPlugin, TablePlugin, ImagePlugin, CodeBlockPlugin, PlaceholderPlugin,
];
const EMPTY: Value = [{ type: KEYS.p, children: [{ text: "" }] }];

/** Plate nodes for text that may contain template variables. */
function textWithChips(text: string) {
  const nodes: (TElement | { text: string })[] = [];
  let at = 0;
  for (const match of text.matchAll(PLACEHOLDER_PATTERN)) {
    if (match.index! > at) nodes.push({ text: text.slice(at, match.index) });
    nodes.push({ type: PLACEHOLDER, name: match[0], children: [{ text: "" }] });
    at = match.index! + match[0].length;
  }
  if (at < text.length) nodes.push({ text: text.slice(at) });
  return nodes;
}

export default function RichTextValueEditor({ value, readOnly, onChange, insertRef }: { value: string; readOnly: boolean; onChange: (value: string) => void; insertRef?: Ref<{ insertText(value: string): void }> }) {
  useTranslation("typed-domain");
  const changeRef = useRef(onChange);
  changeRef.current = onChange;
  // What the editor currently holds as stored HTML; equal means "nothing to report".
  const lastHtml = useRef(value);
  const initial = useRef<{ visual: boolean; nodes: Value }>(undefined);
  if (!initial.current) {
    const nodes = roundTrips(value, sanitizeRichText) ? htmlToValue(sanitizeRichText(value)) : null;
    initial.current = { visual: !!nodes, nodes: nodes ?? EMPTY };
  }
  const [mode, setMode] = useState<"visual" | "source">(initial.current.visual ? "visual" : "source");
  const [unsupported, setUnsupported] = useState(!initial.current.visual);
  const [source, setSource] = useState(value);
  const sourceRef = useRef<{ insertText(value: string): void }>(null);
  const editor = usePlateEditor({ plugins, value: initial.current.nodes, override: { components } });

  const emit = (html: string) => {
    if (html === lastHtml.current) return;
    lastHtml.current = html;
    changeRef.current(html);
  };
  // Opening content must not report a change; only real edits do.
  const visualHtml = useRef(valueToHtml(initial.current.nodes));
  const onValueChange = ({ value: next }: { value: Value }) => {
    const html = valueToHtml(next);
    if (html === visualHtml.current) return;
    visualHtml.current = html;
    emit(sanitizeRichText(html));
  };

  // A new value from outside (e.g. a template switch) replaces the content.
  useEffect(() => {
    if (value === lastHtml.current) return;
    lastHtml.current = value;
    setSource(value);
    const nodes = roundTrips(value, sanitizeRichText) ? htmlToValue(sanitizeRichText(value)) : null;
    if (nodes) {
      visualHtml.current = valueToHtml(nodes);
      editor.tf.setValue(nodes);
      setUnsupported(false);
    } else {
      setUnsupported(true);
      setMode("source");
    }
  }, [value, editor]);

  useImperativeHandle(insertRef, () => ({ insertText(text: string) {
    if (readOnly) return;
    if (mode === "source") { sourceRef.current?.insertText(text); return; }
    if (!editor.selection) editor.tf.select(editor.api.end([]));
    editor.tf.focus();
    if (isPlaceholder(text)) editor.tf.insertNodes({ type: PLACEHOLDER, name: text, children: [{ text: "" }] } as TElement);
    else editor.tf.insertNodes(textWithChips(text) as TElement[]);
  } }), [readOnly, mode, editor]);

  const switchTo = (next: "visual" | "source") => {
    if (next === mode) return;
    if (next === "source") { setSource(lastHtml.current); setMode("source"); return; }
    const nodes = roundTrips(source, sanitizeRichText) ? htmlToValue(sanitizeRichText(source)) : null;
    if (!nodes) { setUnsupported(true); return; }
    visualHtml.current = valueToHtml(nodes);
    editor.tf.setValue(nodes);
    setUnsupported(false);
    setMode("visual");
  };

  return <div className="overflow-hidden rounded-md border border-border bg-background text-foreground">
    <Plate editor={editor} readOnly={readOnly} onValueChange={onValueChange}>
      <Toolbar editor={editor} mode={mode} readOnly={readOnly} onMode={switchTo} />
      {mode === "source" && unsupported && <InlineAlert className="m-2">{typedText("richtext.sourceOnly")}</InlineAlert>}
      {mode === "visual"
        ? <PlateContent aria-label={typedText("richValue")} aria-multiline="true" role="textbox"
          className="min-h-60 max-h-[32rem] overflow-y-auto px-3 py-2 text-sm leading-relaxed outline-none [&_*]:break-words" />
        : <div className="h-60"><CodeEditor insertRef={sourceRef} value={source} language="html" path="file:///rich-text-source.html" label={typedText("richtext.source")} readOnly={readOnly}
          onChange={next => { setSource(next); emit(sanitizeRichText(next)); }} /></div>}
    </Plate>
  </div>;
}

function ToolbarButton({ label, active, disabled, onClick, children }: { label: string; active?: boolean; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <Button type="button" variant={active ? "secondary" : "ghost"} size="icon" className="size-8" title={label} aria-label={label}
    {...(active === undefined ? {} : { "aria-pressed": active })} disabled={disabled}
    // Keep the editor's selection: the button acts on it.
    onMouseDown={event => event.preventDefault()} onClick={onClick}>{children}</Button>;
}

const MARK_ICONS: Record<Mark, [ReactNode, Parameters<typeof typedText>[0]]> = {
  bold: [<Bold key="b" />, "richtext.bold"], italic: [<Italic key="i" />, "richtext.italic"], underline: [<Underline key="u" />, "richtext.underline"],
  strikethrough: [<Strikethrough key="s" />, "richtext.strikethrough"], code: [<Code key="c" />, "richtext.inlineCode"],
};
const BLOCKS: [string, Parameters<typeof typedText>[0]][] = [
  [KEYS.p, "richtext.paragraph"], [KEYS.h1, "richtext.heading1"], [KEYS.h2, "richtext.heading2"], [KEYS.h3, "richtext.heading3"],
  [KEYS.blockquote, "richtext.quote"], [KEYS.codeBlock, "richtext.codeBlock"],
];
const ALIGN_ICONS: Record<(typeof ALIGNMENTS)[number], [ReactNode, Parameters<typeof typedText>[0]]> = {
  left: [<AlignLeft key="l" />, "richtext.alignLeft"], center: [<AlignCenter key="c" />, "richtext.alignCenter"],
  right: [<AlignRight key="r" />, "richtext.alignRight"], justify: [<AlignJustify key="j" />, "richtext.alignJustify"],
};

function Toolbar({ editor, mode, readOnly, onMode }: { editor: PlateEditor; mode: "visual" | "source"; readOnly: boolean; onMode: (mode: "visual" | "source") => void }) {
  const marks = useEditorSelector(current => current.api.marks() ?? {}, []);
  const block = useEditorSelector(current => current.api.block({ highest: true })?.[0] as TElement | undefined, []);
  const off = readOnly || mode !== "visual";
  const blockType = block?.type === KEYS.ulClassic || block?.type === KEYS.olClassic ? block.type : block?.type ?? KEYS.p;
  const align = String(block?.align ?? "left") as (typeof ALIGNMENTS)[number];
  // Slate copies the browser selection into the editor with a short delay; a
  // toolbar click right after selecting (e.g. Ctrl+A) must not act on a stale one.
  const sync = () => {
    const dom = window.getSelection();
    const root = editor.api.toDOMNode(editor);
    if (!dom?.rangeCount || !root || !dom.anchorNode || !root.contains(dom.anchorNode)) return;
    const range = editor.api.toSlateRange(dom, { exactMatch: false, suppressThrow: true });
    if (range) editor.tf.select(range);
  };
  const act = (run: () => void) => () => { sync(); run(); };
  const selection = useRef(editor.selection);
  const remember = () => { sync(); selection.current = editor.selection; };
  const restore = () => { if (selection.current) editor.tf.select(selection.current); editor.tf.focus(); };
  const setBlock = (type: string) => {
    if (type === KEYS.codeBlock) toggleCodeBlock(editor as SlateEditor);
    else editor.tf.toggleBlock(type);
    editor.tf.focus();
  };
  return <div role="toolbar" aria-label={typedText("richtext.toolbar")} className="flex flex-wrap items-center gap-0.5 border-b border-border bg-card p-1">
    <ToolbarButton label={typedText("richtext.undo")} disabled={off} onClick={act(() => editor.undo())}><Undo2 /></ToolbarButton>
    <ToolbarButton label={typedText("richtext.redo")} disabled={off} onClick={act(() => editor.redo())}><Redo2 /></ToolbarButton>
    <DropdownMenu>
      <DropdownMenuTrigger asChild disabled={off}>
        <Button type="button" variant="ghost" size="sm" className="h-8 min-w-24 justify-between" aria-label={typedText("richtext.blockType")} onMouseDown={event => { event.preventDefault(); sync(); }}>
          {typedText(BLOCKS.find(([type]) => type === blockType)?.[1] ?? "richtext.paragraph")}<ChevronDown />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent onCloseAutoFocus={event => event.preventDefault()}>
        {BLOCKS.map(([type, label]) => <DropdownMenuItem key={type} onSelect={() => setBlock(type)}>{typedText(label)}</DropdownMenuItem>)}
      </DropdownMenuContent>
    </DropdownMenu>
    {MARKS.map(mark => <ToolbarButton key={mark} label={typedText(MARK_ICONS[mark][1])} active={!!(marks as Record<string, unknown>)[mark]} disabled={off}
      onClick={act(() => { editor.tf.toggleMark(mark); editor.tf.focus(); })}>{MARK_ICONS[mark][0]}</ToolbarButton>)}
    <ToolbarButton label={typedText("richtext.bulletedList")} active={blockType === KEYS.ulClassic} disabled={off} onClick={act(() => { toggleList(editor as SlateEditor, { type: KEYS.ulClassic }); editor.tf.focus(); })}><List /></ToolbarButton>
    <ToolbarButton label={typedText("richtext.numberedList")} active={blockType === KEYS.olClassic} disabled={off} onClick={act(() => { toggleList(editor as SlateEditor, { type: KEYS.olClassic }); editor.tf.focus(); })}><ListOrdered /></ToolbarButton>
    <DropdownMenu>
      <DropdownMenuTrigger asChild disabled={off}>
        <Button type="button" variant="ghost" size="icon" className="size-8" aria-label={typedText("richtext.align")} title={typedText("richtext.align")} onMouseDown={event => { event.preventDefault(); sync(); }}>{ALIGN_ICONS[align]?.[0] ?? <AlignLeft />}</Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent onCloseAutoFocus={event => event.preventDefault()}>
        {ALIGNMENTS.map(value => <DropdownMenuItem key={value} onSelect={() => { setAlign(editor as SlateEditor, value); editor.tf.focus(); }}>{ALIGN_ICONS[value][0]}{typedText(ALIGN_ICONS[value][1])}</DropdownMenuItem>)}
      </DropdownMenuContent>
    </DropdownMenu>
    <LinkButton editor={editor} disabled={off} remember={remember} restore={restore} />
    <ImageButton editor={editor} disabled={off} remember={remember} restore={restore} />
    <ToolbarButton label={typedText("richtext.table")} disabled={off} onClick={act(() => { insertTable(editor as SlateEditor, { rowCount: 3, colCount: 3 }, { select: true }); editor.tf.focus(); })}><Table /></ToolbarButton>
    <div className="ml-auto">
      <Button type="button" variant={mode === "source" ? "secondary" : "ghost"} size="sm" className="h-8" aria-pressed={mode === "source"}
        onClick={() => onMode(mode === "source" ? "visual" : "source")}>
        <CodeXml />{typedText(mode === "source" ? "richtext.visual" : "richtext.source")}
      </Button>
    </div>
  </div>;
}

function FieldPopover({ label, icon, disabled, remember, children }: { label: string; icon: ReactNode; disabled: boolean; remember: () => void; children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  return <Popover.Root open={open} onOpenChange={next => { if (next) remember(); setOpen(next); }}>
    <Popover.Trigger asChild disabled={disabled}>
      <Button type="button" variant="ghost" size="icon" className="size-8" aria-label={label} title={label} onMouseDown={event => event.preventDefault()}>{icon}</Button>
    </Popover.Trigger>
    <Popover.Portal>
      <Popover.Content align="start" sideOffset={4} className="z-50 w-72 space-y-2 rounded-md border border-border bg-popover p-3 text-popover-foreground shadow-md">
        {children(() => setOpen(false))}
      </Popover.Content>
    </Popover.Portal>
  </Popover.Root>;
}

function LinkButton({ editor, disabled, remember, restore }: { editor: PlateEditor; disabled: boolean; remember: () => void; restore: () => void }) {
  const current = useEditorSelector(ed => ed.api.above({ match: { type: KEYS.link } })?.[0] as TElement | undefined, []);
  const [url, setUrl] = useState("");
  const [newTab, setNewTab] = useState(false);
  useEffect(() => { setUrl(String(current?.url ?? "")); setNewTab(!!current?.target); }, [current]);
  return <FieldPopover label={typedText("richtext.link")} icon={<Link2 />} disabled={disabled} remember={remember}>
    {close => <AutoForm className="space-y-2" onSubmit={event => {
      event.preventDefault();
      restore();
      if (url.trim()) upsertLink(editor as SlateEditor, { url: url.trim(), ...(newTab ? { target: "_blank" } : {}) });
      close();
    }}>
      <Input aria-label={typedText("richtext.linkUrl")} placeholder="https://" value={url} onChange={event => setUrl(event.target.value)} autoFocus />
      <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={newTab} onChange={event => setNewTab(event.target.checked)} />{typedText("richtext.linkNewTab")}</label>
      <div className="flex justify-end gap-2">
        {current && <Button type="button" variant="outline" size="sm" onClick={() => { restore(); unwrapLink(editor as SlateEditor); close(); }}>{typedText("richtext.linkRemove")}</Button>}
        <Button type="submit" size="sm">{typedText("richtext.apply")}</Button>
      </div>
    </AutoForm>}
  </FieldPopover>;
}

function ImageButton({ editor, disabled, remember, restore }: { editor: PlateEditor; disabled: boolean; remember: () => void; restore: () => void }) {
  const [url, setUrl] = useState("");
  const [alt, setAlt] = useState("");
  return <FieldPopover label={typedText("richtext.image")} icon={<ImageIcon />} disabled={disabled} remember={remember}>
    {close => <AutoForm className="space-y-2" onSubmit={event => {
      event.preventDefault();
      if (!url.trim()) return;
      restore();
      editor.tf.insertNodes({ type: KEYS.img, url: url.trim(), ...(alt.trim() ? { alt: alt.trim() } : {}), children: [{ text: "" }] } as TElement);
      setUrl(""); setAlt(""); close();
    }}>
      <Input aria-label={typedText("richtext.imageUrl")} placeholder="https://" value={url} onChange={event => setUrl(event.target.value)} autoFocus />
      <Input aria-label={typedText("richtext.imageAlt")} value={alt} onChange={event => setAlt(event.target.value)} />
      <div className="flex justify-end"><Button type="submit" size="sm">{typedText("richtext.apply")}</Button></div>
    </AutoForm>}
  </FieldPopover>;
}
