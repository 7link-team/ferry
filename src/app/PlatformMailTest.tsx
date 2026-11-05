import {useState} from "react";
import {api,errorText} from "../shared/api/api";
import {commonText as t} from "../shared/i18n/i18n";
import {AutoField} from "../shared/forms/auto-form";
import {Input} from "../shared/ui/input";
import {Button} from "../shared/ui/button";

/** Tests the saved configuration, then reloads its verification state/version. */
export function PlatformMailTest({readOnly,onVerified,draft}:{readOnly:boolean;draft:Record<string,unknown>;onVerified:(value:Record<string,unknown>)=>void}) {
  const [email,setEmail]=useState("");const [busy,setBusy]=useState(false);const [message,setMessage]=useState("");
  return <AutoField label={t("resource-domain:platform.mailTestRecipient")} help={t("resource-domain:platform.mailTestHelp")}><div className="space-y-2">
    <div className="flex flex-wrap gap-2"><Input aria-label={t("resource-domain:platform.mailTestRecipient")} className="min-w-0 flex-1" type="email" value={email} disabled={readOnly||busy} onChange={e=>setEmail(e.target.value)}/>
    <Button type="button" variant="outline" disabled={readOnly||busy||!email.trim()} onClick={async()=>{
      setBusy(true);setMessage("");try {
        const saved=await api<Record<string,unknown>>("/platform/settings/1");
        if(["mail_enabled","mail_host","mail_port","mail_security","mail_from","mail_username","mail_code_subject","mail_code_body","mail_test_subject","mail_test_body"].some(key=>String(saved[key]??"")!==String(draft[key]??""))||draft.mail_password)throw new Error(t("resource-domain:platform.mailSaveFirst"));
        onVerified(await api<Record<string,unknown>>("/platform/mail-test",{method:"POST",body:JSON.stringify({email})}));
      }catch(e){setMessage(errorText(e));}finally{setBusy(false);}
    }}>{t(busy?"common-domain:common.loading":"resource-domain:platform.mailTest")}</Button></div>
    {message&&<p role="alert" className="text-sm text-destructive">{message}</p>}
  </div></AutoField>;
}
