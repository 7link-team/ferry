import { useRef, useState } from "react";
import { commonText } from "../shared/i18n/i18n";
import { AutoField } from "../shared/forms/auto-form";
import { Input } from "../shared/ui/input";
import { Button } from "../shared/ui/button";

const t=(key:string)=>commonText(`resource-domain:platform.${key}`);
export function PlatformMailTemplates({draft,readOnly,onPatch}:{draft:Record<string,unknown>;readOnly:boolean;onPatch:(value:Record<string,unknown>)=>void}) {
  const [kind,setKind]=useState("code");
  const bodyRef=useRef<HTMLTextAreaElement>(null);
  const subjectKey=`mail_${kind}_subject`,bodyKey=`mail_${kind}_body`;
  const subject=String(draft[subjectKey]??""),body=String(draft[bodyKey]??"");
  const defaultSubject=t(kind==="code"?"mailDefaultCodeSubject":"mailDefaultTestSubject");
  const defaultBody=t(kind==="code"?"mailDefaultCodeBody":"mailDefaultTestBody");
  const preview=(value:string)=>value.replace(/\{\{(code|email|expires_minutes)\}\}/g,(_all,key:string)=>({code:"123456",email:"user@example.com",expires_minutes:"10"})[key]!);
  return <div className="min-w-0 space-y-5">
    <div className="flex flex-wrap items-end justify-between gap-3"><AutoField label={t("mailTemplateKind")}><select className="h-9 rounded-md border bg-background px-3 text-sm" value={kind} onChange={event=>setKind(event.target.value)}><option value="code">{t("mailCodeTemplate")}</option><option value="test">{t("mailTestTemplate")}</option></select></AutoField>
      {!readOnly&&<Button type="button" variant="outline" onClick={()=>onPatch({[subjectKey]:"",[bodyKey]:""})}>{t("mailTemplateReset")}</Button>}</div>
    <AutoField label={t("mailTemplateSubject")}><Input maxLength={200} value={subject} placeholder={defaultSubject} readOnly={readOnly} onChange={event=>onPatch({[subjectKey]:event.target.value})}/></AutoField>
    <AutoField label={t("mailTemplateBody")}><textarea ref={bodyRef} className="min-h-44 w-full rounded-md border border-input bg-background px-3 py-2 text-sm" maxLength={16384} value={body} placeholder={defaultBody} readOnly={readOnly} onChange={event=>onPatch({[bodyKey]:event.target.value})}/></AutoField>
    {!readOnly&&<div className="flex flex-wrap items-center gap-2"><span className="text-sm text-muted-foreground">{t("mailTemplateVariables")}</span>{(kind==="code"?["code","email","expires_minutes"]:["email"]).map(key=><Button key={key} type="button" size="sm" variant="outline" onClick={()=>{const input=bodyRef.current,start=input?.selectionStart??body.length,end=input?.selectionEnd??body.length,token=`{{${key}}}`;onPatch({[bodyKey]:body.slice(0,start)+token+body.slice(end)});requestAnimationFrame(()=>{input?.focus();input?.setSelectionRange(start+token.length,start+token.length);});}}>{`{{${key}}}`}</Button>)}</div>}
    <p className="text-sm text-muted-foreground">{t("mailTemplateHelp")}</p>
    <section aria-label={t("mailTemplatePreview")} className="min-w-0 space-y-3 border-t pt-4"><h3 className="font-medium">{t("mailTemplatePreview")}</h3><p className="break-words font-medium">{preview(subject.trim()?subject:defaultSubject)}</p><p className="whitespace-pre-wrap break-words text-sm">{preview(body.trim()?body:defaultBody)}</p></section>
  </div>;
}
