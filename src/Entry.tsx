import { commonText, useCommonTranslation } from "./i18n";
import { AutoForm, AutoField } from "./components/auto-form";
/**
 * Register / sign in / create or join a workspace.
 *
 * Rendered in place of the shell, on whatever URL the visitor arrived at —
 * never by redirecting to a login route. That is what makes a shared deep
 * link work: sign in and the same URL renders its target page, with its
 * filters and scroll offset intact (§5.3).
 */
import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, ApiError, errorText } from "./api";
import { navigate, parse } from "./router";

type Step = "setup" | "sign-in" | "register" | "workspace";

type SetupStatus = { required: boolean; has_users: boolean; registration_enabled: boolean };
// Remove the local sidecar proof before any API request; never persist it.
let desktopSetupToken = new URLSearchParams(location.hash.slice(1)).get("setup") ?? "";
if (desktopSetupToken) history.replaceState(history.state, "", location.pathname + location.search);

export function Entry({ onReady, error: outerError, signedIn = false, initialize = false }: { onReady: () => void; error: string; signedIn?: boolean; initialize?: boolean }) {
  useCommonTranslation();
  const [step, setStep] = useState<Step>(initialize ? "setup" : signedIn ? "workspace" : "sign-in");
  const [setup, setSetup] = useState<SetupStatus>();
  const [token, setToken] = useState(desktopSetupToken);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    void api<SetupStatus>("/setup", { cache: "no-store" }).then(status => {
      if (!live) return;
      setSetup(status);
      if (!status.required) { desktopSetupToken = ""; setToken(""); }
      if (status.required && (!signedIn || initialize)) setStep("setup");
    }).catch(cause => { if (live) setError(errorText(cause)); });
    return () => { live = false; };
  }, [signedIn, initialize, attempt]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(outerError);

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await work();
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  };

  const fields = (form: HTMLFormElement) =>
    Object.fromEntries(new FormData(form).entries()) as Record<string, string>;

  return (
    <div className="grid h-full place-items-center p-6">
      <div className="w-full max-w-sm space-y-5">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Ferry</h1>
          <p className="text-sm text-muted-foreground">
            {step === "workspace"
              ? commonText("auth.workspaceIntro")
              : commonText("auth.tagline")}
          </p>
        </div>

        {error && (
          <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </p>
        )}

        {!setup && <div role="status"><p>{commonText("auth.checking")}</p><Button variant="outline" onClick={() => setAttempt(n => n + 1)}>{commonText("auth.recheck")}</Button></div>}

        {setup && step === "setup" && (
          <AutoForm className="space-y-4" onSubmit={event => {
            event.preventDefault();
            const body = fields(event.currentTarget);
            void run(async () => {
              if (body.password !== body.confirm_password) throw new Error(commonText("auth.passwordMismatch"));
              try {
                await api("/setup", { method: "POST", body: JSON.stringify({ email: body.email, name: body.name, password: body.password, token }) });
              } catch (cause) {
                if (cause instanceof ApiError && cause.status === 409) {
                  const status = await api<SetupStatus>("/setup", { cache: "no-store" });
                  if (!status.required) {
                    desktopSetupToken = "";
                    setToken("");
                    setSetup(status);
                    setStep("sign-in");
                  }
                }
                throw cause;
              }
              desktopSetupToken = "";
              setToken("");
              navigate({ workspace: "_platform", group: "platform", item: "workspaces", mode: "list" }, {}, { replace: true });
              onReady();
            });
          }}>
            <h2 className="text-lg font-semibold">{commonText("auth.setupTitle")}</h2>
            <p className="text-sm text-muted-foreground">{commonText("auth.setupHelp")}</p>
            {setup.has_users && <p className="text-sm text-muted-foreground">{commonText("auth.existingHelp")}</p>}
            {!desktopSetupToken && <AutoField label={commonText("auth.setupToken")} name="token" required help={commonText("auth.setupTokenHelp")}>
              <Input id="setup-token" name="token" type="password" value={token} onChange={e => setToken(e.target.value)} required autoComplete="off" />
            </AutoField>}
            <AutoField label={commonText("auth.name")} className="block space-y-1 text-sm"><Input name="name" required={!setup.has_users} autoComplete="name" /></AutoField>
            <AutoField label={commonText("auth.email")} className="block space-y-1 text-sm"><Input name="email" type="email" required autoComplete="username" /></AutoField>
            <AutoField label={commonText("auth.password")} name="password" required help={commonText("auth.setupPasswordHelp")}><Input id="setup-password" name="password" type="password" required minLength={8} autoComplete="new-password" /></AutoField>
            <AutoField label={commonText("auth.confirmPassword")} className="block space-y-1 text-sm"><Input name="confirm_password" type="password" required minLength={8} autoComplete="new-password" /></AutoField>
            <Button type="submit" className="w-full" disabled={busy}>{busy && <Loader2 className="animate-spin" />}{commonText("auth.initialize")}</Button>
            {signedIn ? <Button type="button" variant="link" className="w-full" disabled={busy} onClick={onReady}>{commonText("auth.deferSetup")}</Button>
              : setup.has_users && <Button type="button" variant="link" className="w-full" disabled={busy} onClick={() => { setStep("sign-in"); setError(""); }}>{commonText("auth.existingSignIn")}</Button>}
          </AutoForm>
        )}

        {setup && step === "sign-in" && (
          <AutoForm
            className="space-y-3"
            columns={1}
            fields={{
              email: { name: commonText("auth.email"), type: "email", required: true, config: { autoComplete: "username", placeholder: commonText("auth.email") } },
              password: { name: commonText("auth.password"), type: "password", required: true, config: { autoComplete: "current-password", placeholder: commonText("auth.password") } },
            }}
            onSubmit={(event) => {
              event.preventDefault();
              const body = fields(event.currentTarget);
              void run(async () => {
                await api("/sessions", { method: "POST", body: JSON.stringify(body) });
                onReady();
              });
            }}
          >
            <Button type="submit" className="w-full" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />}{commonText("auth.signIn")}</Button>
            {setup.registration_enabled && <Button
              type="button"
              variant="link"
              className="w-full"
              onClick={() => (setStep("register"), setError(""))}
            >
              {commonText("auth.registerPrompt")}</Button>}
            {setup.required && <Button type="button" variant="link" className="w-full" onClick={() => setStep("setup")}>{commonText("auth.setupTitle")}</Button>}
          </AutoForm>
        )}

        {step === "register" && (
          <AutoForm
            className="space-y-3"
            columns={1}
            fields={{
              name: { name: commonText("auth.name"), type: "text", required: true, config: { autoComplete: "name", placeholder: commonText("auth.name") } },
              email: { name: commonText("auth.email"), type: "email", required: true, config: { autoComplete: "username", placeholder: commonText("auth.email") } },
              password: { name: commonText("auth.password"), type: "password", required: true, help: commonText("auth.minPassword"), config: { minLength: 8, autoComplete: "new-password", placeholder: commonText("auth.passwordPlaceholder") } },
            }}
            onSubmit={(event) => {
              event.preventDefault();
              const body = fields(event.currentTarget);
              void run(async () => {
                await api("/users", { method: "POST", body: JSON.stringify(body) });
                // Registration signs you in, but you have no workspace yet.
                setStep("workspace");
              });
            }}
          >
            <Button type="submit" className="w-full" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />}{commonText("auth.register")}</Button>
            <Button
              type="button"
              variant="link"
              className="w-full"
              onClick={() => (setStep("sign-in"), setError(""))}
            >
              {commonText("auth.signInPrompt")}</Button>
          </AutoForm>
        )}

        {step === "workspace" && (
          <div className="space-y-6">
            <AutoForm
              className="space-y-3"
              onSubmit={(event) => {
                event.preventDefault();
                const body = fields(event.currentTarget);
                void run(async () => {
                  const created = await api<{ id: string }>("/workspaces", {
                    method: "POST",
                    body: JSON.stringify(body),
                  });
                  enter(created.id);
                  onReady();
                });
              }}
            >
              <p className="text-sm font-medium">{commonText("auth.createWorkspace")}</p>
              <AutoField label={commonText("common.name")} name="name" required><Input name="name" placeholder={commonText("auth.workspaceNamePlaceholder")} required /></AutoField>
              <AutoField label={commonText("auth.slug")} name="slug" required help={commonText("auth.slugHelp")}><Input
                name="slug"
                placeholder={commonText("auth.slugPlaceholder")}
                required
                pattern="[a-z0-9][a-z0-9-]{0,38}[a-z0-9]"
              /></AutoField>
              <Button type="submit" className="w-full" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}{commonText("common.create")}</Button>
            </AutoForm>

            <AutoForm
              className="space-y-3 border-t border-border pt-5"
              onSubmit={(event) => {
                event.preventDefault();
                const body = fields(event.currentTarget);
                void run(async () => {
                  const joined = await api<{ id: string }>("/workspace-memberships", {
                    method: "POST",
                    body: JSON.stringify(body),
                  });
                  enter(joined.id);
                  onReady();
                });
              }}
            >
              <p className="text-sm font-medium">{commonText("auth.joinWorkspace")}</p>
              <AutoField label={commonText("auth.invitation")} name="code" required><Input name="code" placeholder={commonText("auth.invitation")} required className="uppercase" /></AutoField>
              <Button type="submit" variant="outline" className="w-full" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}{commonText("auth.join")}</Button>
            </AutoForm>
          </div>
        )}
      </div>
    </div>
  );
}

/** Only rewrites the workspace segment, so a deep link keeps its destination. */
function enter(workspaceId: string) {
  navigate({ ...parse(), workspace: workspaceId }, Object.fromEntries(new URLSearchParams(location.search)), { replace: true });
}
