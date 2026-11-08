import { useQueryValue } from "./router";
import { lazy, Suspense, useMemo, useRef } from "react";
import { commonText } from "../shared/i18n/i18n";
import { AutoField } from "../shared/forms/auto-form";
import { Input } from "../shared/ui/input";
import { Button } from "../shared/ui/button";
import { Tabs, TabsList, TabsTrigger } from "../shared/ui/tabs";
import { LoadingSkeleton } from "../shared/ui/loading-skeleton";
const CodeEditor = lazy(() => import("../shared/editor/CodeEditor").then(module=>({default:module.CodeEditor})));
import { mailPreview, plainMailHtml } from "../shared/mail/template";

const RichTextEditor = lazy(() => import("../features/maintenance/RichTextValueEditor"));
const t = (key: string) => commonText(`resource-domain:platform.${key}`);
export function PlatformMailTemplates({draft,readOnly,onPatch}:{draft:Record<string,unknown>;readOnly:boolean;onPatch:(value:Record<string,unknown>)=>void}) {
  const [templateKind,setKind] = useQueryValue("mail_template", "code");
  const [editorMode,setMode] = useQueryValue("mail_editor", "rich");
  const kind = templateKind === "test" ? "test" : "code";
  const mode = editorMode === "html" ? "html" : "rich";
  const subjectKey = `mail_${kind}_subject`, bodyKey = `mail_${kind}_body`, htmlKey = `mail_${kind}_html`;
  const defaultSubject = t(kind === "code" ? "mailDefaultCodeSubject" : "mailDefaultTestSubject");
  const defaultBody = t(kind === "code" ? "mailDefaultCodeBody" : "mailDefaultTestBody");
  const subject = String(draft[subjectKey] || defaultSubject);
  const body = String(draft[htmlKey] || plainMailHtml(String(draft[bodyKey] || defaultBody)));
  const subjectInput = useRef<HTMLInputElement>(null);
  const editor = useRef<{ insertText(value: string): void }>(null);
  const insertionTarget = useRef<"subject" | "body">("body");
  const preview = useMemo(() => mailPreview(subject, body), [subject, body]);
  const insertVariable = (key: string) => {
    if (readOnly) return;
    const token = `{{${key}}}`;
    const input = subjectInput.current;
    if (insertionTarget.current === "subject" && input) {
      const start = input.selectionStart ?? subject.length;
      const end = input.selectionEnd ?? start;
      const next = subject.slice(0, start) + token + subject.slice(end);
      if (next.length > 200) return;
      onPatch({ [subjectKey]: next });
      requestAnimationFrame(() => { input.focus(); input.setSelectionRange(start + token.length, start + token.length); });
    } else editor.current?.insertText(token);
  };
  return <div className="min-w-0 space-y-4" data-mail-template-workspace>
    <div className="flex flex-wrap items-end justify-between gap-3"><AutoField label={t("mailTemplateKind")}><select className="h-9 rounded-md border bg-background px-3 text-sm" value={kind} onChange={event=>setKind(event.target.value)}><option value="code">{t("mailCodeTemplate")}</option><option value="test">{t("mailTestTemplate")}</option></select></AutoField>
      {!readOnly && <Button type="button" variant="outline" onClick={()=>onPatch({[subjectKey]:"",[bodyKey]:"",[htmlKey]:""})}>{t("mailTemplateReset")}</Button>}</div>
    <div className="grid min-w-0 items-start gap-5 xl:grid-cols-2" data-mail-template-columns>
    <section className="min-w-0 space-y-4" aria-label={t("mailTemplates")}>
    <AutoField label={t("mailTemplateSubject")}><Input ref={subjectInput} maxLength={200} value={subject} readOnly={readOnly} onFocus={()=>{insertionTarget.current="subject";}} onChange={event=>onPatch({[subjectKey]:event.target.value})}/></AutoField>
    {!readOnly && <div className="flex flex-wrap items-center gap-2"><span className="text-sm text-muted-foreground">{t("mailTemplateVariables")}</span>{(kind==="code"?["code","email","expires_minutes"]:["email"]).map(key=><Button key={key} type="button" className="min-h-11 break-all font-mono text-xs" variant="outline" onMouseDown={event=>event.preventDefault()} onClick={()=>insertVariable(key)}>{`{{${key}}}`}</Button>)}</div>}
    <div onFocusCapture={()=>{insertionTarget.current="body";}}>
    <AutoField label={t("mailTemplateBody")} help={t("mailTemplateHelp")}><Tabs value={mode} onValueChange={setMode}><TabsList><TabsTrigger value="rich">{t("mailRichEditor")}</TabsTrigger><TabsTrigger value="html">HTML</TabsTrigger></TabsList>
      {mode === "rich" ? <Suspense fallback={<LoadingSkeleton variant="editor" />}><RichTextEditor key={kind} insertRef={editor} value={body} readOnly={readOnly} onChange={value=>onPatch({[htmlKey]:value})} /></Suspense> : <div className="h-96 min-w-0 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" />}><CodeEditor insertRef={editor} value={body} language="html" path={`file:///mail-${kind}.html`} label={t("mailTemplateBody")} readOnly={readOnly} onChange={value=>onPatch({[htmlKey]:value})} /></Suspense></div>}
    </Tabs></AutoField>
    </div>
    </section>
    <section aria-label={t("mailTemplatePreview")} className="min-w-0 overflow-hidden rounded-lg border xl:sticky xl:top-4"><h3 className="border-b bg-muted/30 px-4 py-3 text-sm font-medium">{t("mailTemplatePreview")}</h3><iframe title={t("mailTemplatePreview")} sandbox="" referrerPolicy="no-referrer" srcDoc={preview} className="block h-[32rem] w-full bg-white" /></section>
    </div>
  </div>;
}
