import { useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { Input } from "../../shared/ui/input";

export function Invitation({ workspace }: { workspace: string }) {
  useCommonTranslation();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [messageKey, setMessageKey] = useState("");
  const load = async (rotate: boolean) => {
    if (rotate && !window.confirm(commonText("invitation.rotateConfirm")))
      return;
    setBusy(true);
    setMessage("");
    setMessageKey("");
    try {
      const value = await api<{ code: string }>(
        `/workspaces/${workspace}/members/invitation`,
        { method: rotate ? "POST" : "GET", cache: "no-store" },
      );
      setCode(value.code);
      if (rotate) setMessageKey("invitation.rotated");
    } catch (cause) {
      setMessage(errorText(cause));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="mb-3 space-y-2 rounded-md border p-3"
      aria-label={commonText("invitation.title")}
    >
      <div className="flex flex-wrap items-end gap-2">
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => void load(false)}
        >
          {commonText("invitation.view")}
        </Button>
        {code && (
          <>
            <AutoField label={commonText("auth.invitation")} className="text-sm">
              <Input readOnly value={code} />
            </AutoField>
            <Button
              variant="outline"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(code);
                  setMessage("");
                  setMessageKey("invitation.copied");
                } catch {
                  setMessage("");
                  setMessageKey("invitation.copyFailed");
                }
              }}
            >
              {commonText("invitation.copy")}
            </Button>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => void load(true)}
            >
              {commonText("invitation.rotate")}
            </Button>
          </>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        {commonText("invitation.help")}
      </p>
      {(message || messageKey) && (
        <p role="status" className="text-sm">
          {messageKey ? commonText(messageKey) : message}
        </p>
      )}
    </section>
  );
}
