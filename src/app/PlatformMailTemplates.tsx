import { useQueryValue } from "./router";
import { lazy, Suspense, useMemo, useRef } from "react";
import { commonText, i18n } from "../shared/i18n/i18n";
import { currentLocale, localeNames, supportedLocales, type Locale } from "../shared/i18n/locale";
import { AutoField } from "../shared/forms/auto-form";
import { Input } from "../shared/ui/input";
import { Button } from "../shared/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../shared/ui/tabs";
import { LoadingSkeleton } from "../shared/ui/loading-skeleton";
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from "../shared/ui/dropdown-menu";
import { ChevronDown } from "lucide-react";
import { PlatformMailTest } from "./PlatformMailTest";
const CodeEditor = lazy(() => import("../shared/editor/CodeEditor").then(module=>({default:module.CodeEditor})));
import { mailPreview, plainMailHtml } from "../shared/mail/template";

const RichTextEditor = lazy(() => import("../features/maintenance/RichTextValueEditor"));
const t = (key: string) => commonText(`resource-domain:platform.${key}`);
export function PlatformMailTemplates({draft,readOnly,onPatch}:{draft:Record<string,unknown>;readOnly:boolean;onPatch:(value:Record<string,unknown>)=>void}) {
  const [selectedLocale,setLocale] = useQueryValue("mail_locale", currentLocale());
  const [editorMode,setMode] = useQueryValue("mail_editor", "rich");
  const locale: Locale = supportedLocales.includes(selectedLocale as Locale) ? selectedLocale as Locale : currentLocale();
  const mode = editorMode === "html" ? "html" : "rich";
  const templates = (draft.mail_code_templates ?? {}) as Record<string,{subject?:string;body?:string;html?:string}>;
  const template = templates[locale] ?? {subject:String(draft.mail_code_subject ?? ""),body:String(draft.mail_code_body ?? ""),html:String(draft.mail_code_html ?? "")};
  const patchTemplate = (value: Partial<typeof template>) => onPatch({mail_code_templates:{...templates,[locale]:{...template,...value}}});
  const defaultSubject = String(i18n.t("platform.mailDefaultCodeSubject",{ns:"resource-domain",lng:locale,keySeparator:false}));
  const defaultBody = String(i18n.t("platform.mailDefaultCodeBody",{ns:"resource-domain",lng:locale,keySeparator:false}));
  const subject = template.subject || defaultSubject;
  const body = template.html || plainMailHtml(template.body || defaultBody);
  const subjectInput = useRef<HTMLInputElement>(null);
  const editor = useRef<{ insertText(value: string): void }>(null);
  const insertionTarget = useRef<"subject" | "body">("body");
  const inserted = useRef(false);
  const preview = useMemo(() => mailPreview(subject, body), [subject, body]);
  const insertVariable = (key: string) => {
    if (readOnly) return;
    inserted.current = true;
    const token = `{{${key}}}`;
    const input = subjectInput.current;
    if (insertionTarget.current === "subject" && input) {
      const start = input.selectionStart ?? subject.length;
      const end = input.selectionEnd ?? start;
      const next = subject.slice(0, start) + token + subject.slice(end);
      if (next.length > 200) return;
      patchTemplate({subject:next});
      requestAnimationFrame(() => { input.focus(); input.setSelectionRange(start + token.length, start + token.length); });
    } else editor.current?.insertText(token);
  };
  return <div className="min-w-0 space-y-4" data-mail-template-workspace>
    <div className="flex flex-wrap items-end justify-between gap-3"><AutoField label={t("mailTemplateLanguage")} help={t("mailTemplateLanguageHelp")}><select className="min-h-11 max-w-full rounded-md border bg-background px-3 text-sm" value={locale} onChange={event=>setLocale(event.target.value)}>{supportedLocales.map(value=><option key={value} value={value}>{localeNames[value]}</option>)}</select></AutoField>
      {!readOnly && <Button type="button" variant="outline" onClick={()=>patchTemplate({subject:"",body:"",html:""})}>{t("mailTemplateReset")}</Button>}</div>
    <div className="grid min-w-0 items-start gap-5 xl:grid-cols-2" data-mail-template-columns>
    <section className="min-w-0 space-y-4" aria-label={t("mailTemplates")}>
    <AutoField label={t("mailTemplateSubject")}><Input ref={subjectInput} maxLength={200} value={subject} readOnly={readOnly} onFocus={()=>{insertionTarget.current="subject";}} onChange={event=>patchTemplate({subject:event.target.value})}/></AutoField>
    <AutoField label={t("mailTemplateBody")} help={t("mailTemplateHelp")}><Tabs value={mode} onValueChange={setMode}>
      <div className="flex flex-wrap items-center justify-between gap-2" data-mail-editor-toolbar><TabsList><TabsTrigger value="rich">{t("mailRichEditor")}</TabsTrigger><TabsTrigger value="html">HTML</TabsTrigger></TabsList>
      {!readOnly && <DropdownMenu onOpenChange={open=>{if(open)inserted.current=false;}}><DropdownMenuTrigger asChild><Button type="button" variant="outline" className="ml-auto min-h-11">{t("mailTemplateVariables")}<ChevronDown aria-hidden="true" className="size-4" /></Button></DropdownMenuTrigger><DropdownMenuContent align="end" className="w-80 max-w-[calc(100vw-2rem)]" onCloseAutoFocus={event=>{if(inserted.current)event.preventDefault();}}>{["code","email","expires_minutes"].map(key=><DropdownMenuItem key={key} className="min-h-11 flex-col items-start gap-1 whitespace-normal" onSelect={()=>insertVariable(key)}><span className="font-mono text-sm">{`{{${key}}}`}</span><span className="text-xs text-muted-foreground">{t(`mailVariable_${key}`)}</span></DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu>}
      </div>
      <div onFocusCapture={()=>{insertionTarget.current="body";}}>
      {mode === "rich" ? <Suspense fallback={<LoadingSkeleton variant="editor" />}><RichTextEditor key={locale} insertRef={editor} value={body} readOnly={readOnly} onChange={value=>patchTemplate({html:value})} /></Suspense> : <div className="h-96 min-w-0 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" />}><CodeEditor key={locale} insertRef={editor} value={body} language="html" path={`file:///mail-code-${locale}.html`} label={t("mailTemplateBody")} readOnly={readOnly} onChange={value=>patchTemplate({html:value})} /></Suspense></div>}
      </div>
    </Tabs></AutoField>
    {!readOnly && <PlatformMailTest key={locale} draft={draft} readOnly={readOnly} onVerified={onPatch} locale={locale} template={{subject:template.subject ?? "",body:template.body ?? "",html:template.html ?? ""}} />}
    </section>
    <section aria-label={t("mailTemplatePreview")} className="min-w-0 overflow-hidden rounded-lg border xl:sticky xl:top-4"><h3 className="border-b bg-muted/30 px-4 py-3 text-sm font-medium">{t("mailTemplatePreview")}</h3><iframe title={t("mailTemplatePreview")} sandbox="" referrerPolicy="no-referrer" srcDoc={preview} className="block h-[32rem] w-full bg-white" /></section>
    </div>
  </div>;
}
