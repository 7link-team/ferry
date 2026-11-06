import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { ArrowDown,ArrowUp,Plus,Trash2,Upload } from "lucide-react";
import { lazy,Suspense,useEffect,useRef,useState } from "react";
import { useTranslation } from "react-i18next";
import { api,errorText,localizedFetch } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText,i18n,typedText } from "../../shared/i18n/i18n";
import { currentLocale, localeNames, supportedLocales } from "../../shared/i18n/locale";
import { useTheme } from "../../shared/theme/theme";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";
import { TagBadge } from "../tags/TagBadge";
export { sanitizeRichText } from "./value-content";

const Editor = lazy(async () => { await import("../../shared/editor/runtime"); return import("@monaco-editor/react"); });
const RichTextEditor = lazy(() => import("./RichTextValueEditor"));
const MarkdownPreview = lazy(() => import("./MarkdownValuePreview"));
export const VALUE_TYPES = [
  ["text", "文本"], ["num", "数字"], ["tab", "标签选项"], ["selection", "选择项"],
  ["image", "图片"], ["video", "视频"], ["file", "文件"], ["code", "代码"],
  ["markdown", "Markdown"], ["html", "HTML"], ["richtext", "富文本"], ["json", "JSON"],
] as const;
export const valueTypeOptions = VALUE_TYPES.map(([value, label]) => ({ value, label, labelKey: `typed-domain:type.${value}` }));
export interface TypedValueRow extends Record<string, unknown> {
  id: string; key: string; name: string; description: string; builtin: boolean;
  category: string; value_type: string; value_json: string; schema_json: string;
  items: string; value: number; version: number;
}
interface Props {
  workspace: string; resource: "settings" | "dictionaries"; draft: Partial<TypedValueRow>;
  readOnly: boolean; onPatch: (patch: Partial<TypedValueRow>) => void;
  onValidityChange: (blocked: boolean) => void;
}
export function parseJson<T>(raw: unknown, fallback: T): T {
  try { return JSON.parse(String(raw)) as T; } catch { return fallback; }
}
const isOptions = (type: string) => type === "tab" || type === "selection";
const isMedia = (type: string) => ["image", "video", "file"].includes(type);
export function initialValue(type: string) {
  return JSON.stringify(type === "num" ? 0 : type === "json" ? {} : isOptions(type) || isMedia(type) ? [] : "");
}
export function valueOf(draft: Partial<TypedValueRow>) {
  if (draft.value_json !== undefined) return parseJson<unknown>(draft.value_json, null);
  return draft.items !== undefined ? parseJson<unknown>(draft.items, []) : draft.value ?? "";
}
export function typedValueError(type: string, raw: string, schema: Record<string, unknown>): string {
  let value: unknown;
  try { value = JSON.parse(raw); } catch { return typedText("invalidJson"); }
  if (type === "num") {
    if (typeof value !== "number" || !Number.isFinite(value)) return typedText("invalidNumber");
    if (schema.min !== undefined && value < Number(schema.min)) return typedText("minimum", { value: schema.min });
    if (schema.max !== undefined && value > Number(schema.max)) return typedText("maximum", { value: schema.max });
    if (schema.decimals !== undefined && Math.abs(value * 10 ** Number(schema.decimals) - Math.round(value * 10 ** Number(schema.decimals))) > 1e-7) return typedText("decimalLimit", { value: schema.decimals });
  } else if (isOptions(type)) {
    if (!Array.isArray(value)) return typedText("optionsArray");
    const keys = new Set<string>();
    for (const item of value as Record<string, unknown>[]) {
      const key = String(item?.value ?? item?.key ?? "");
      if (!key.trim() || !String(item?.label ?? "").trim()) return typedText("optionRequired");
      if (keys.has(key)) return typedText("optionDuplicate");
      if (item.color !== undefined && (typeof item.color !== "string" || !/^#([\da-f]{3}|[\da-f]{6})$/i.test(item.color))) return typedText("optionRequired");
      if (item.labels !== undefined && (!item.labels || typeof item.labels !== "object" || Array.isArray(item.labels) || Object.entries(item.labels).some(([locale, label]) => !supportedLocales.some(supported => supported === locale) || typeof label !== "string" || !label.trim() || [...label].length > 100 || label.includes("\0")))) return typedText("optionRequired");
      keys.add(key);
    }
  } else if (isMedia(type)) {
    if (!Array.isArray(value)) return typedText("filesArray");
    if (Number(schema.maxFiles) > 0 && value.length > Number(schema.maxFiles)) return typedText("fileLimit", { value: schema.maxFiles });
    if (Number(schema.maxSizeMb) > 0 && value.some(file => Number(file?.size) > Number(schema.maxSizeMb) * 1024 * 1024)) return typedText("fileSize", { value: schema.maxSizeMb });
  } else if (type !== "json") {
    if (typeof value !== "string") return typedText("textRequired");
    if (Number(schema.maxLength) > 0 && [...value].length > Number(schema.maxLength)) return typedText("characterLimit", { value: schema.maxLength });
  }
  if (schema.min !== undefined && schema.max !== undefined && Number(schema.min) > Number(schema.max)) return typedText("invalidBounds");
  if (schema.minSelect !== undefined && Number(schema.maxSelect) > 0 && Number(schema.minSelect) > Number(schema.maxSelect)) return typedText("invalidSelectionBounds");
  return "";
}

export function TypedSchemaEditor({ draft, readOnly, onPatch }: Props) {
  useTranslation("typed-domain");
  const type = draft.value_type ?? "text";
  const schema = parseJson<Record<string, unknown>>(draft.schema_json, {});
  const update = (key: string, value: string, numeric = true) => {
    const next = { ...schema };
    if (value === "") delete next[key]; else next[key] = numeric ? Number(value) : value;
    onPatch({ schema_json: JSON.stringify(next) });
  };
  const numeric = (key: string, label: string, min?: number, max?: number) => <AutoField key={key} label={label} help={typedText("constraintHelp")}>
    <Input type="number" aria-label={label} value={String(schema[key] ?? "")} min={min} max={max} disabled={readOnly} onChange={event => update(key, event.target.value)} />
  </AutoField>;
  return <div className="grid min-w-0 gap-4 sm:grid-cols-2">
    {["text", "html", "markdown", "richtext"].includes(type) && numeric("maxLength", typedText("maxLength"), 0)}
    {type === "num" && <>{numeric("min", typedText("min"))}{numeric("max", typedText("max"))}{numeric("decimals", typedText("decimals"), 0, 10)}</>}
    {isOptions(type) && <>{numeric("minSelect", typedText("minSelect"), 0)}{numeric("maxSelect", typedText("maxSelect"), 0)}</>}
    {isMedia(type) && <>{numeric("maxFiles", typedText("maxFiles"), 0)}{numeric("maxSizeMb", typedText("maxSize"), 0)}
      <AutoField label={typedText("accept")} help={typedText("acceptHelp")}><Input aria-label={typedText("accept")} value={String(schema.accept ?? "")} disabled={readOnly} onChange={event => update("accept", event.target.value, false)} /></AutoField></>}
    {type === "code" && <AutoField label={typedText("language")}><select aria-label={typedText("language")} className="h-9 w-full rounded-md border bg-background px-3 text-sm" disabled={readOnly} value={String(schema.lang ?? "text")} onChange={event => update("lang", event.target.value, false)}>{["text", "javascript", "typescript", "python", "go", "rust", "sql", "html", "css", "json", "yaml", "shell"].map(lang => <option key={lang}>{lang}</option>)}</select></AutoField>}
    {type === "json" && <p className="text-sm text-muted-foreground">{typedText("jsonHelp")}</p>}
    {draft.builtin && <p className="text-sm text-muted-foreground sm:col-span-2">{typedText("builtinHelp")}</p>}
  </div>;
}

export function TypedValueEditor(props: Props) {
  useTranslation("typed-domain");
  const { draft, resource, readOnly, onPatch, onValidityChange } = props;
  const type = draft.value_type ?? (resource === "settings" ? "num" : "selection");
  const schema = parseJson<Record<string, unknown>>(draft.schema_json, {});
  const value = valueOf(draft);
  const raw = draft.value_json ?? JSON.stringify(value);
  const [rawError, setRawError] = useState<"" | "invalidNumber">("");
  const [uploadBusy, setUploadBusy] = useState(false);
  const error = rawError ? typedText(rawError) : typedValueError(type, raw, schema);
  const reportBlocked = !readOnly && (!!error || uploadBusy);
  useEffect(() => { onValidityChange(reportBlocked); return () => onValidityChange(false); }, [reportBlocked, onValidityChange]);
  useEffect(() => { setRawError(""); }, [type]);
  const emit = (next: unknown) => {
    setRawError("");
    const patch: Partial<TypedValueRow> = { value_json: JSON.stringify(next) };
    if (isOptions(type)) patch.items = JSON.stringify(next);
    if (type === "num" && typeof next === "number") patch.value = next;
    onPatch(patch);
  };
  const text = typeof value === "string" ? value : "";
  return <div className="min-w-0 space-y-3">
    {isOptions(type) ? <OptionsEditor value={value} readOnly={readOnly} onChange={emit} />
      : isMedia(type) ? <MediaEditor {...props} type={type} schema={schema} value={value} onChange={emit} onValidityChange={setUploadBusy} />
      : type === "num" ? <AutoField label={typedText("value")}><Input aria-label={typedText("value")} type="number" value={typeof value === "number" ? value : ""} min={schema.min as number | undefined} max={schema.max as number | undefined} step={schema.decimals === undefined ? "any" : 10 ** -Number(schema.decimals)} disabled={readOnly} onChange={event => { if (!event.target.value || !Number.isFinite(event.target.valueAsNumber)) { setRawError("invalidNumber"); onPatch({ value_json: "null" }); } else emit(event.target.valueAsNumber); }} /></AutoField>
      : type === "json" ? <CodeValue label={typedText("jsonValue")} value={raw} language="json" readOnly={readOnly} onChange={next => { setRawError(""); onPatch({ value_json: next }); }} />
      : type === "code" ? <CodeValue label={typedText("codeValue")} value={text} language={String(schema.lang ?? "text")} readOnly={readOnly} onChange={emit} />
      : type === "richtext" ? <AutoField label={typedText("richValue")}><Suspense fallback={<LoadingSkeleton variant="editor" label={typedText("loadingRich")} />}><RichTextEditor value={text} readOnly={readOnly} onChange={emit} /></Suspense></AutoField>
      : <AutoField label={typedText("value")}><textarea aria-label={typedText("value")} className="min-h-36 w-full rounded-md border border-input bg-background px-3 py-2 text-sm" value={text} maxLength={Number(schema.maxLength) || undefined} disabled={readOnly} onChange={event => emit(event.target.value)} /></AutoField>}
    {["markdown", "html", "richtext"].includes(type) && <details open={readOnly}><summary className="cursor-pointer text-sm font-medium">{typedText("preview")}</summary><SafePreview value={text} markdown={type === "markdown"} /></details>}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
  </div>;
}

function OptionsEditor({ value, readOnly, onChange }: { value: unknown; readOnly: boolean; onChange: (value: unknown) => void }) {
  useTranslation("typed-domain");
  const items = Array.isArray(value) ? value as Record<string, unknown>[] : [];
  const [labelLocale, setLabelLocale] = useState(() => items.some(item => item.labels && typeof item.labels === "object") ? i18n.resolvedLanguage ?? currentLocale() : "");
  const move = (index: number, offset: number) => { const next = [...items]; [next[index], next[index + offset]] = [next[index + offset]!, next[index]!]; onChange(next); };
  const update = (index: number, key: string, text: string) => onChange(items.map((item, i) => {
    if (i !== index) return item;
    if (key === "label" && labelLocale) {
      const labels = { ...(item.labels as Record<string, string> ?? {}) };
      if (text.trim()) labels[labelLocale] = text; else delete labels[labelLocale];
      return { ...item, labels };
    }
    const next = { ...item, value: String(item.value ?? item.key ?? ""), label: String(item.label ?? ""), [key]: text };
    if (key === "color" && !text) delete next.color;
    return next;
  }));
  return <div className="space-y-3" aria-label={typedText("options")}>
    <div className="flex flex-wrap items-center justify-between gap-3"><h3 className="text-sm font-medium">{typedText("options")}</h3>{!readOnly && <Button type="button" variant="outline" size="sm" onClick={() => onChange([...items, { value: "", label: "" }])}><Plus className="size-4" />{typedText("addOption")}</Button>}</div>
    <AutoField label={typedText("optionLocale")} help={labelLocale ? typedText("optionLocaleHelp") : undefined}><select className="h-9 w-full max-w-64 rounded-md border bg-background px-3 text-sm" aria-label={typedText("optionLocale")} value={labelLocale} onChange={event => setLabelLocale(event.target.value)}><option value="">{typedText("optionDefaultLabel")}</option>{supportedLocales.map(locale => <option key={locale} value={locale}>{localeNames[locale]}</option>)}</select></AutoField>
    {!items.length && <p className="text-sm text-muted-foreground">{typedText("emptyOptions")}</p>}
    {items.map((item, index) => <div key={index} className="flex flex-wrap items-end gap-2 rounded-md border p-3">
      <AutoField label={typedText("optionValue", { index: index + 1 })} className="min-w-32 flex-1"><Input aria-label={typedText("optionValue", { index: index + 1 })} value={String(item.value ?? item.key ?? "")} disabled={readOnly} onChange={event => update(index, "value", event.target.value)} /></AutoField>
      <AutoField label={typedText("optionLabel", { index: index + 1 })} className="min-w-40 flex-1"><Input aria-label={typedText("optionLabel", { index: index + 1 })} value={labelLocale ? (item.labels as Record<string,string> | undefined)?.[labelLocale] ?? "" : String(item.label ?? "")} placeholder={labelLocale ? String(item.label ?? "") : undefined} maxLength={100} disabled={readOnly} onChange={event => update(index, "label", event.target.value)} /></AutoField>
      <AutoField label={commonText("resource-domain:tag.color")} className="min-w-0 max-w-full"><div className="flex h-10 min-w-0 items-center gap-2"><input type="color" aria-label={`${typedText("optionLabel", { index: index + 1 })} ${commonText("resource-domain:tag.color")}`} className="h-10 w-12 cursor-pointer rounded-md border border-input bg-background p-1 disabled:cursor-default" value={/^#[\da-f]{6}$/i.test(String(item.color ?? "")) ? String(item.color) : /^#[\da-f]{3}$/i.test(String(item.color ?? "")) ? `#${String(item.color).slice(1).split("").map(char => char + char).join("")}` : "#4f9cf9"} disabled={readOnly} onChange={event => update(index, "color", event.target.value)} /><TagBadge color={typeof item.color === "string" ? item.color : undefined}><span className="max-w-32 truncate">{String(item.label ?? "") || typedText("optionLabel", { index: index + 1 })}</span></TagBadge>{!readOnly && item.color !== undefined && <Button type="button" size="sm" variant="ghost" aria-label={`${commonText("common.clear")} ${typedText("optionLabel", { index: index + 1 })} ${commonText("resource-domain:tag.color")}`} onClick={() => update(index, "color", "")}>{commonText("common.clear")}</Button>}</div></AutoField>
      {!readOnly && <div className="flex gap-1"><Button type="button" variant="outline" size="icon" aria-label={typedText("moveUp", { index: index + 1 })} disabled={!index} onClick={() => move(index, -1)}><ArrowUp className="size-4" /></Button><Button type="button" variant="outline" size="icon" aria-label={typedText("moveDown", { index: index + 1 })} disabled={index === items.length - 1} onClick={() => move(index, 1)}><ArrowDown className="size-4" /></Button><Button type="button" variant="ghost" size="icon" aria-label={typedText("deleteOption", { index: index + 1 })} onClick={() => onChange(items.filter((_, i) => i !== index))}><Trash2 className="size-4" /></Button></div>}
    </div>)}
  </div>;
}

function CodeValue({ label, value, language, readOnly, onChange }: { label: string; value: string; language: string; readOnly: boolean; onChange: (value: string) => void }) {
  useTranslation("typed-domain");
  const theme = useTheme();
  const [advanced, setAdvanced] = useState(false);
  return <div className="space-y-2"><div className="flex flex-wrap items-center justify-between gap-3"><span className="text-sm font-medium">{label}</span><Button type="button" size="sm" variant="outline" onClick={() => setAdvanced(!advanced)}>{advanced ? typedText("plainEditor") : typedText("codeEditor")}</Button></div><AutoField label={label} hideLabel>{advanced ? <div className="h-80 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" label={typedText("loadingEditor")} />}><Editor loading={<LoadingSkeleton variant="editor" label={typedText("loadingEditor")} />} value={value} language={language === "text" ? "plaintext" : language} theme={theme.resolved === "dark" ? "vs-dark" : "vs"} onChange={next => onChange(next ?? "")} options={{ ariaLabel: label, readOnly, automaticLayout: true, minimap: { enabled: false }, wordWrap: "on", scrollBeyondLastLine: false, occurrencesHighlight: "off" }} /></Suspense></div> : <textarea aria-label={label} className="min-h-48 w-full rounded-md border bg-background p-3 font-mono text-sm" value={value} disabled={readOnly} spellCheck={false} onChange={event => onChange(event.target.value)} />}</AutoField></div>;
}

function SafePreview({ value, markdown }: { value: string; markdown?: boolean }) {
  useTranslation("typed-domain");
  if (markdown) return <Suspense fallback={<LoadingSkeleton variant="editor" label={typedText("loadingPreview")} />}><MarkdownPreview value={value} /></Suspense>;
  return <iframe title={typedText("contentPreview")} sandbox="" referrerPolicy="no-referrer" className="mt-2 min-h-64 w-full rounded-md border bg-white" srcDoc={`<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data: blob:; font-src 'none'; form-action 'none'; base-uri 'none'"><style>body{font:14px/1.6 system-ui;padding:16px;overflow-wrap:anywhere}img,video{max-width:100%}pre{white-space:pre-wrap}</style></head><body>${value}</body></html>`} />;
}

interface UploadedFile { id: string; url: string; name: string; size: number; content_type: string; }
function MediaEditor({ workspace, resource, type, schema, value, readOnly, onChange, onValidityChange }: Props & { type: string; schema: Record<string, unknown>; value: unknown; onChange: (value: unknown) => void }) {
  useTranslation("typed-domain");
  const input = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | { key: Parameters<typeof typedText>[0]; values?: Record<string, unknown> }>("");
  const files = Array.isArray(value) ? value as UploadedFile[] : [];
  const maxFiles = schema.maxFiles === undefined ? 1 : Number(schema.maxFiles) === 0 ? Infinity : Number(schema.maxFiles);
  const accept = String(schema.accept ?? (type === "image" ? "image/*" : type === "video" ? "video/*" : ""));
  useEffect(() => { onValidityChange(busy); return () => onValidityChange(false); }, [busy, onValidityChange]);
  const upload = async (selected: File[]) => {
    setError("");
    if (selected.length + files.length > maxFiles) { setError({ key: "fileLimit", values: { value: maxFiles } }); return; }
    if (Number(schema.maxSizeMb) > 0 && selected.some(file => file.size > Number(schema.maxSizeMb) * 1024 * 1024)) { setError({ key: "fileSize", values: { value: schema.maxSizeMb } }); return; }
    if (type === "image" && selected.some(file => !file.type.startsWith("image/")) || type === "video" && selected.some(file => !file.type.startsWith("video/"))) { setError({ key: type === "image" ? "chooseImage" : "chooseVideo" }); return; }
    setBusy(true);
    const next = [...files];
    try {
      for (const file of selected) { const body = new FormData(); body.append("file", file); next.push(await api<UploadedFile>(`/workspaces/${workspace}/value-files?resource=${resource}`, { method: "POST", body })); onChange([...next]); }
    } catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); if (input.current) input.current.value = ""; }
  };
  return <div className="space-y-3"><div className="flex flex-wrap items-center gap-3"><span className="text-sm">{typedText("attachments", { count: files.length, limit: Number.isFinite(maxFiles) ? maxFiles : typedText("unlimited") })}</span>{!readOnly && <><input ref={input} type="file" hidden aria-label={typedText("chooseFile")} multiple={maxFiles > 1} accept={accept} onChange={event => void upload(Array.from(event.target.files ?? []))} /><Button type="button" variant="outline" disabled={busy || files.length >= maxFiles} onClick={() => input.current?.click()}><Upload className="size-4" />{busy ? typedText("uploading") : typedText("upload")}</Button></>}</div>
    {files.map((file, index) => <div key={file.id ?? index} className="flex min-w-0 items-center gap-3 rounded-md border p-3"><FilePreview file={file} type={type} /><a href={safeFileUrl(file.url)} className="min-w-0 flex-1 break-all text-sm text-primary underline" download={file.name}>{file.name || typedText("attachment")}</a><span className="text-xs text-muted-foreground">{Math.ceil(Number(file.size ?? 0) / 1024)} KB</span>{!readOnly && <Button type="button" size="icon" variant="ghost" disabled={busy} aria-label={typedText("removeFile", { index: index + 1 })} onClick={() => onChange(files.filter((_, i) => i !== index))}><Trash2 className="size-4" /></Button>}</div>)}
    {error && <p role="alert" className="text-sm text-destructive">{typeof error === "string" ? error : typedText(error.key, error.values)}</p>}
  </div>;
}
function safeFileUrl(value: unknown): string | undefined { const raw = String(value ?? ""); return /^\/api\/workspaces\/[^/]+\/value-files\/[^/?#]+$/.test(raw) ? raw : undefined; }
function FilePreview({ file, type }: { file: UploadedFile; type: string }) {
  useTranslation("typed-domain");
  const [url, setUrl] = useState("");
  useEffect(() => {
    const source = safeFileUrl(file.url);
    if (!source || !["image", "video"].includes(type)) return;
    let live = true; let objectUrl = "";
    void localizedFetch(source, { credentials: "same-origin" }).then(response => { if (!response.ok) throw new Error(); return response.arrayBuffer(); }).then(bytes => { if (!live) return; const contentType = String(file.content_type ?? ""); if (type === "image" && !/^image\/(png|jpeg|gif|webp|avif|bmp)$/.test(contentType) || type === "video" && !/^video\/(mp4|webm|ogg)$/.test(contentType)) return; objectUrl = URL.createObjectURL(new Blob([bytes], { type: contentType })); setUrl(objectUrl); }).catch(() => {});
    return () => { live = false; if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [file.url, file.content_type, type]);
  return url ? type === "image" ? <img src={url} alt={file.name || typedText("filePreview")} className="size-16 rounded object-contain" /> : <video src={url} controls preload="metadata" className="h-24 max-w-40 rounded" /> : null;
}
