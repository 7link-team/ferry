import { AutoField,AutoForm } from "../shared/forms/auto-form";
import { commonText,useCommonTranslation } from "../shared/i18n/i18n";
/**
 * Register / sign in / create or join a workspace.
 *
 * Unauthenticated deep links use the login route's validated redirect query.
 * App restores the destination after authentication; setup and workspace
 * creation continue to use this same entry form.
 */
import { Loader2 } from "lucide-react";
import { Dialog } from "radix-ui";
import { useEffect,useRef,useState } from "react";
import { api,ApiError,errorText } from "../shared/api/api";
import { Button } from "../shared/ui/button";
import { Input } from "../shared/ui/input";
import { navigate,useRoute } from "./router";
import { LoginChallenge, type LoginChallengeData, type LoginProof } from "./LoginChallenge";

type Step = "setup" | "sign-in" | "register" | "workspace";

type SetupStatus = { required: boolean; has_users: boolean; registration_enabled: boolean; email_verification_required?:boolean };
// Remove the local sidecar proof before any API request; never persist it.
let desktopSetupToken = new URLSearchParams(location.hash.slice(1)).get("setup") ?? "";
if (desktopSetupToken) history.replaceState(history.state, "", location.pathname + location.search);

export function Entry({ onReady, error: outerError, signedIn = false, initialize = false }: { onReady: () => void; error: string; signedIn?: boolean; initialize?: boolean }) {
  useCommonTranslation();
  const route = useRoute();
  const [step, setStepState] = useState<Step>(initialize || route.workspace === "_initialize" ? "setup" : signedIn ? "workspace" : route.workspace === "_register" ? "register" : "sign-in");
  const setStep = (value: Step) => {
    setStepState(value);
    navigate({workspace:({setup:"_initialize","sign-in":"_login",register:"_register",workspace:"_workspaces"})[value]}, {});
  };
  useEffect(()=>{setStepState(initialize || route.workspace === "_initialize" ? "setup" : signedIn ? "workspace" : route.workspace === "_register" ? "register" : "sign-in");},[route.workspace,signedIn,initialize]);
  const [setup, setSetup] = useState<SetupStatus>();
  const [token, setToken] = useState(desktopSetupToken);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    void api<SetupStatus>("/setup", { cache: "no-store" }).then(status => {
      if (!live) return;
      setSetup(status);
      if (!status.required) { desktopSetupToken = ""; setToken(""); }
      if (status.required && (!signedIn || initialize)) {setStepState("setup");navigate({workspace:"_initialize"}, {}, {replace:true});}
    }).catch(cause => { if (live) setError(errorText(cause)); });
    return () => { live = false; };
  }, [signedIn, initialize, attempt]);
  const [busy, setBusy] = useState(false);
  const [challenge,setChallenge]=useState<LoginChallengeData>();
  const [proof,setProof]=useState<LoginProof>({});
  const [loginEmail,setLoginEmail]=useState("");
  const [pendingLogin,setPendingLogin]=useState<Record<string,string>>();
  const [verificationError,setVerificationError]=useState("");
  const [verified,setVerified]=useState(false);
  const verificationInFlight=useRef(false);
  const refreshChallenge=async(email:string)=>{
    const value=await api<LoginChallengeData>("/sessions/challenge",{method:"POST",body:JSON.stringify({email})});
    setProof({});setChallenge(value.required?value:undefined);
    setVerified(false);
    return value;
  };
  const [error, setError] = useState(outerError);
  const [codeBusy,setCodeBusy]=useState(false);
  const [codeRetryAt,setCodeRetryAt]=useState(0);
  const [codeSeconds,setCodeSeconds]=useState(0);
  const [codeSent,setCodeSent]=useState(false);
  useEffect(()=>{if(!codeRetryAt)return;const update=()=>setCodeSeconds(Math.max(0,Math.ceil((codeRetryAt-Date.now())/1000)));update();const timer=setInterval(update,1000);return()=>clearInterval(timer);},[codeRetryAt]);

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

  const closeChallenge=()=>{setChallenge(undefined);setPendingLogin(undefined);setProof({});setVerificationError("");setVerified(false);};
  const signIn=async(body:Record<string,string>,verificationToken?:string)=>{
    try {
      await api("/sessions",{method:"POST",body:JSON.stringify({...body,...(verificationToken?{login_verification_token:verificationToken}:{})})});
      closeChallenge();onReady();
    }catch(cause){
      if(cause instanceof ApiError && cause.status===428){
        const value=await refreshChallenge(body.email??"");
        if(value.required){setLoginEmail(body.email??"");setPendingLogin(body);setVerificationError(verificationToken?commonText("resource-domain:loginGuard.retryChallenge"):"");return;}
      }
      closeChallenge();throw cause;
    }
  };
  const verifyChallenge=(candidate:LoginProof=proof)=>{
    if(busy||verificationInFlight.current||!pendingLogin)return;
    verificationInFlight.current=true;
    void run(async()=>{
      setVerificationError("");
      let result:{login_verification_token:string};
      try{
        result=await api("/sessions/challenge/verify",{method:"POST",body:JSON.stringify({email:loginEmail,...candidate})});
      }catch(cause){
        setVerificationError(errorText(cause));
        if(cause instanceof ApiError && cause.status===428){
          try{await refreshChallenge(loginEmail);setVerificationError(commonText("resource-domain:loginGuard.retryChallenge"));}catch(refreshError){setVerificationError(errorText(refreshError));}
        }
        return;
      }
      setVerified(true);
      await signIn(pendingLogin,result.login_verification_token);
    }).finally(()=>{verificationInFlight.current=false;});
  };

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

        {!setup && error && <Button variant="outline" onClick={() => { setError(""); setAttempt(n => n + 1); }}>{commonText("auth.recheck")}</Button>}

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

        {step === "sign-in" && (
          <AutoForm
            className="space-y-3"
            columns={1}
            onChange={event=>{const input=event.target;if(input instanceof HTMLInputElement && input.name==="email" && input.value!==loginEmail){setLoginEmail(input.value);setProof({});setChallenge(undefined);}}}
            fields={{
              email: { name: commonText("auth.email"), type: "email", required: true, config: { autoComplete: "username", placeholder: commonText("auth.email") } },
              password: { name: commonText("auth.password"), type: "password", required: true, config: { autoComplete: "current-password", placeholder: commonText("auth.password") } },
            }}
            onSubmit={(event) => {
              event.preventDefault();
              const body = fields(event.currentTarget);
              void run(async () => {
                const email=body.email??"";
                setLoginEmail(email);
                setVerificationError("");
                const value=await refreshChallenge(email);
                if(value.required){setPendingLogin(body);return;}
                await signIn(body);
              });
            }}
          >
            <Button type="submit" className="w-full" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />}{commonText("auth.signIn")}</Button>
            {setup?.registration_enabled && <Button
              type="button"
              variant="link"
              className="w-full"
              onClick={() => (setStep("register"), setError(""))}
            >
              {commonText("auth.registerPrompt")}</Button>}
            {setup?.required && <Button type="button" variant="link" className="w-full" onClick={() => setStep("setup")}>{commonText("auth.setupTitle")}</Button>}
          </AutoForm>
        )}

        {challenge?.required && pendingLogin && <Dialog.Root open onOpenChange={open=>{if(!open&&!busy)closeChallenge();}}><Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
          <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(25rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 space-y-4 dialog-shell rounded-md border bg-background p-5 shadow-lg">
            <Dialog.Title className="text-base font-semibold">{commonText("resource-domain:loginGuard.title")}</Dialog.Title>
            <Dialog.Description className="text-sm text-muted-foreground">{commonText("resource-domain:loginGuard.dialogHelp")}</Dialog.Description>
            <div className="dialog-body space-y-4">
            {verificationError&&<p role="alert" className="text-sm text-destructive">{verificationError}</p>}
            <LoginChallenge challenge={challenge} onProof={setProof} onVerify={verifyChallenge} busy={busy} onRefresh={()=>void run(async()=>{setVerificationError("");await refreshChallenge(loginEmail);})} />
            {busy&&<p role="status" className="flex items-center gap-2 text-sm"><Loader2 className="size-4 animate-spin" />{commonText(verified?"resource-domain:loginGuard.verified":"resource-domain:loginGuard.verifying")}</p>}
            </div>
            <div className="dialog-footer">
              <Button type="button" variant="outline" disabled={busy} onClick={closeChallenge}>{commonText("common.cancel")}</Button>
              <Button type="button" disabled={busy||(!proof.login_challenge_answer&&!proof.turnstile_token)} onClick={()=>verifyChallenge()}>{commonText("resource-domain:loginGuard.verify")}</Button>
            </div>
          </Dialog.Content>
        </Dialog.Portal></Dialog.Root>}

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
                try { await api("/users", { method: "POST", body: JSON.stringify(body) }); }
                catch(e) { void api<SetupStatus>("/setup",{cache:"no-store"}).then(setSetup).catch(()=>{});throw e; }
                // Registration signs you in, but you have no workspace yet.
                setStep("workspace");
                onReady();
              });
            }}
          >
            {setup?.email_verification_required&&<AutoField label={commonText("resource-domain:platform.mailCode")} help={commonText("resource-domain:platform.mailCodeHelp")}><div className="space-y-2"><div className="flex gap-2">
              <Input aria-label={commonText("resource-domain:platform.mailCode")} className="min-w-0 flex-1" name="verification_code" required inputMode="numeric" pattern="[0-9]{6}" maxLength={6} autoComplete="one-time-code"/>
              <Button type="button" variant="outline" disabled={codeBusy||codeSeconds>0||busy} onClick={async event=>{
                const email=event.currentTarget.form?.elements.namedItem("email") as HTMLInputElement|null;
                if(!email?.reportValidity())return;
                setCodeBusy(true);setError("");setCodeSent(false);
                try { const response=await api<{retry_after:number}>("/registration-code",{method:"POST",body:JSON.stringify({email:email.value})});setCodeRetryAt(Date.now()+response.retry_after*1000);setCodeSent(true); }
                catch(e){setError(errorText(e));}finally{setCodeBusy(false);}
              }}>{codeSeconds>0?`${codeSeconds}s`:commonText("resource-domain:platform.mailSendCode")}</Button>
            </div>{codeSent&&<p role="status" className="text-sm text-muted-foreground">{commonText("resource-domain:platform.mailCodeSent")}</p>}</div></AutoField>}
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

/** Creating or joining a workspace starts at that workspace's overview. */
function enter(workspaceId: string) {
  navigate({ workspace: workspaceId,group:"dashboard",item:"overview",mode:"list" }, {}, { replace: true });
}
