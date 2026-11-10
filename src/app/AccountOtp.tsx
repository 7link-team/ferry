import { useEffect, useState, type ReactNode } from "react";
import { Dialog } from "radix-ui";
import { Copy, Download, Loader2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { api, ApiError, errorText } from "../shared/api/api";
import { AutoField, AutoForm } from "../shared/forms/auto-form";
import { FormActions } from "../shared/forms/auto-form/form-actions";
import { commonText as t, useCommonTranslation } from "../shared/i18n/i18n";
import { Button } from "../shared/ui/button";
import { Input } from "../shared/ui/input";

type Status = { enabled: boolean; enrollment_enabled: boolean; recovery_remaining: number };
type Enrollment = { secret: string; qr_svg: string };
type Codes = { recovery_codes: string[] };

function SecurityDialog({ title, help, children, footer, close }: { title: string; help: string; children: ReactNode; footer: ReactNode; close: () => void }) {
  return <Dialog.Root open onOpenChange={open => { if (!open) close(); }}><Dialog.Portal>
    <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
    <Dialog.Content className="dialog-shell fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-xl border bg-background p-5 text-foreground shadow-lg">
      <Dialog.Title className="flex items-center gap-2 text-base font-semibold"><ShieldCheck className="size-5 shrink-0" />{title}</Dialog.Title>
      <Dialog.Description className="mt-2 text-sm text-muted-foreground">{help}</Dialog.Description>
      <div className="dialog-body mt-4 space-y-4">{children}</div>
      <FormActions className="dialog-footer mt-4">{footer}</FormActions>
    </Dialog.Content>
  </Dialog.Portal></Dialog.Root>;
}

function CodeInput({ recovery, value, change }: { recovery: boolean; value: string; change: (code: string) => void }) {
  return <AutoField label={t(recovery ? "otp:recoveryCode" : "otp:code")} required>
    <Input aria-label={t(recovery ? "otp:recoveryCode" : "otp:code")} name="code" value={value} onChange={event => change(event.target.value)}
      required autoFocus autoComplete="one-time-code" inputMode={recovery ? "text" : "numeric"}
      maxLength={recovery ? 128 : 6} pattern={recovery ? undefined : "[0-9]{6}"} className="min-h-11 font-mono tracking-wider" />
  </AutoField>;
}

/** Second-factor tokens and secrets live only in component memory. */
export function OtpLogin({ token, onReady, onCancel }: { token: string; onReady: () => void; onCancel: () => void }) {
  useCommonTranslation();
  const [code, setCode] = useState("");
  const [recovery, setRecovery] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [expired, setExpired] = useState(false);
  return <SecurityDialog title={t("otp:loginTitle")} help={t("otp:loginHelp")} close={() => { if (!busy) onCancel(); }} footer={<>
    <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>{t("otp:cancel")}</Button>
    <Button type="submit" form="otp-login" disabled={busy || expired || !code.trim()}>{busy && <Loader2 className="size-4 animate-spin" />}{t("otp:confirm")}</Button>
  </>}>
    <AutoForm id="otp-login" columns={1} onSubmit={event => {
      event.preventDefault(); event.stopPropagation(); if (busy || expired) return;
      setBusy(true); setError("");
      void api("/sessions/otp", { method: "POST", body: JSON.stringify({ token, code: code.trim(), use_recovery: recovery }) })
        .then(onReady).catch(cause => {
          setError(errorText(cause)); setCode("");
          if (cause instanceof ApiError && [410, 429].includes(cause.status)) setExpired(true);
        }).finally(() => setBusy(false));
    }}>
      <CodeInput recovery={recovery} value={code} change={setCode} />
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <Button type="button" variant="link" disabled={busy || expired} className="min-h-11 px-0" onClick={() => { setRecovery(!recovery); setCode(""); }}>{t(recovery ? "otp:useOtp" : "otp:useRecovery")}</Button>
    </AutoForm>
  </SecurityDialog>;
}

export function AccountOtp() {
  useCommonTranslation();
  const [status, setStatus] = useState<Status>();
  const [loadError, setLoadError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [mode, setMode] = useState<"bind" | "reset" | "remove">();
  const [password, setPassword] = useState("");
  const [enrollment, setEnrollment] = useState<Enrollment>();
  const [codes, setCodes] = useState<string[]>();
  const [saved, setSaved] = useState(false);
  const [code, setCode] = useState("");
  const [recovery, setRecovery] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true; setLoadError("");
    void api<Status>("/profile/otp", { cache: "no-store" }).then(value => { if (live) setStatus(value); }).catch(cause => { if (live) setLoadError(errorText(cause)); });
    return () => { live = false; };
  }, [attempt]);
  const close = () => {
    if (busy || (codes && !saved)) return;
    setMode(undefined); setEnrollment(undefined); setCodes(undefined); setPassword(""); setCode(""); setRecovery(false); setError(""); setSaved(false);
  };
  const submit = async () => {
    if (busy) return;
    setBusy(true); setError("");
    try {
      if (mode === "bind" && !enrollment) {
        setEnrollment(await api<Enrollment>("/profile/otp/enrollment", { method: "POST", body: JSON.stringify({ current_password: password }) }));
        setPassword("");
      } else if (mode === "remove") {
        await api("/profile/otp", { method: "DELETE", body: JSON.stringify({ current_password: password, code: code.trim(), use_recovery: recovery }) });
        setMode(undefined); setPassword(""); setCode(""); setRecovery(false); setAttempt(n => n + 1);
      } else {
        const result = await api<Codes>(mode === "bind" ? "/profile/otp/enrollment/confirm" : "/profile/otp/recovery-codes", {
          method: "POST", body: JSON.stringify(mode === "bind" ? { code: code.trim() } : { current_password: password, code: code.trim(), use_recovery: recovery }),
        });
        setCodes(result.recovery_codes); setSaved(false); setEnrollment(undefined); setPassword(""); setCode(""); setAttempt(n => n + 1);
      }
    } catch (cause) {
      setError(errorText(cause));
      if (cause instanceof ApiError && cause.status === 410) { setEnrollment(undefined); setCode(""); }
    } finally { setBusy(false); }
  };
  const title = codes ? "otp:codesTitle" : mode === "remove" ? "otp:remove" : mode === "reset" ? "otp:reset" : "otp:bind";
  const help = codes ? "otp:codesHelp" : mode === "remove" ? "otp:removeHelp" : mode === "reset" ? "otp:resetHelp" : "otp:setupHelp";
  return <div className="space-y-3">
    {loadError ? <div role="alert" className="space-y-2"><p className="text-sm text-destructive">{loadError}</p><Button type="button" variant="outline" onClick={() => setAttempt(n => n + 1)}>{t("otp:retry")}</Button></div>
      : !status ? <div role="status" aria-label={t("otp:loading")} className="h-16 animate-pulse rounded-md bg-muted" />
      : <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1"><p className="text-sm font-medium">{t(status.enabled ? "otp:enabled" : "otp:disabled")}</p>
          <p className="mt-1 text-sm text-muted-foreground">{status.enabled ? t("otp:remaining", { count: status.recovery_remaining }) : !status.enrollment_enabled ? t("otp:unavailable") : t("otp:featureHelp")}</p></div>
        <div className="flex flex-wrap gap-2 [&_button]:min-h-11">
          {status.enabled ? <><Button type="button" variant="outline" onClick={() => setMode("reset")}>{t("otp:reset")}</Button><Button type="button" variant="outline" onClick={() => setMode("remove")}>{t("otp:remove")}</Button></>
            : status.enrollment_enabled && <Button type="button" onClick={() => setMode("bind")}>{t("otp:bind")}</Button>}
        </div>
      </div>}
    {mode && <SecurityDialog title={t(title)} help={t(help)} close={close} footer={codes
      ? <Button type="button" disabled={!saved} onClick={close}>{t("otp:close")}</Button>
      : <><Button type="button" variant="outline" disabled={busy} onClick={close}>{t("otp:cancel")}</Button><Button type="submit" form="otp-manage" disabled={busy}>{busy && <Loader2 className="size-4 animate-spin" />}{t("otp:confirm")}</Button></>}>
      {codes ? <>
        <div className="grid grid-cols-1 gap-2 rounded-md border bg-muted/50 p-3 font-mono text-xs sm:grid-cols-2">{codes.map(value => <code key={value} className="break-all select-all">{value}</code>)}</div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" variant="outline" className="min-h-11" onClick={() => void navigator.clipboard.writeText(codes.join("\n")).then(() => toast.success(t("otp:copied"))).catch(() => toast.error(t("otp:copyFailed")))}><Copy />{t("otp:copy")}</Button>
          <Button type="button" variant="outline" className="min-h-11" onClick={() => {
            const url = URL.createObjectURL(new Blob([codes.join("\n") + "\n"], { type: "text/plain;charset=utf-8" }));
            const link = document.createElement("a"); link.href = url; link.download = "recovery-codes.txt"; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
          }}><Download />{t("otp:download")}</Button>
        </div>
        <label className="flex min-h-11 cursor-pointer items-center gap-3 text-sm"><input type="checkbox" checked={saved} onChange={event => setSaved(event.target.checked)} className="size-4 shrink-0" />{t("otp:saved")}</label>
      </> : <AutoForm id="otp-manage" columns={1} className="space-y-4" onSubmit={event => { event.preventDefault(); event.stopPropagation(); void submit(); }}>
        {!enrollment && <AutoField label={t("otp:password")} required><Input aria-label={t("otp:password")} type="password" autoComplete="current-password" name="current_password" value={password} onChange={event => setPassword(event.target.value)} required maxLength={1024} className="min-h-11" /></AutoField>}
        {enrollment && <>
          <img src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(enrollment.qr_svg)}`} alt={t("otp:bind")} width={240} height={240} className="mx-auto max-w-full rounded-md bg-white p-2" />
          <AutoField label={t("otp:manual")} help={t("otp:secretHelp")}><Input aria-label={t("otp:manual")} readOnly value={enrollment.secret} className="font-mono text-sm" /></AutoField>
        </>}
        {(enrollment || mode !== "bind") && <CodeInput recovery={recovery} value={code} change={setCode} />}
        {mode !== "bind" && <Button type="button" variant="link" className="min-h-11 px-0" disabled={busy} onClick={() => { setRecovery(!recovery); setCode(""); }}>{t(recovery ? "otp:useOtp" : "otp:useRecovery")}</Button>}
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      </AutoForm>}
    </SecurityDialog>}
  </div>;
}
