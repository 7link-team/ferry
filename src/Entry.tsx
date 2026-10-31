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
              ? "创建一个工作空间，或用邀请码加入一个。"
              : "工作空间制运维平台"}
          </p>
        </div>

        {error && (
          <p className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </p>
        )}

        {!setup && <div role="status"><p>正在检查平台初始化状态…</p><Button variant="outline" onClick={() => setAttempt(n => n + 1)}>重新检查</Button></div>}

        {setup && step === "setup" && (
          <AutoForm className="space-y-4" onSubmit={event => {
            event.preventDefault();
            const body = fields(event.currentTarget);
            void run(async () => {
              if (body.password !== body.confirm_password) throw new Error("两次输入的密码不一致。");
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
            <h2 className="text-lg font-semibold">初始化平台超级管理员</h2>
            <p className="text-sm text-muted-foreground">首次使用需要设置一次超管账号，没有默认账号或密码。超管可管理平台及所有工作空间。</p>
            {setup.has_users && <p className="text-sm text-muted-foreground">已有账号可填写原邮箱和密码完成授权；原姓名和密码不会被修改。</p>}
            {!desktopSetupToken && <AutoField label="初始化凭据" name="token" required help="由部署者从服务数据目录的 setup-token 文件读取。">
              <Input id="setup-token" name="token" type="password" value={token} onChange={e => setToken(e.target.value)} required autoComplete="off" />
            </AutoField>}
            <AutoField label="姓名" className="block space-y-1 text-sm"><Input name="name" required={!setup.has_users} autoComplete="name" /></AutoField>
            <AutoField label="邮箱" className="block space-y-1 text-sm"><Input name="email" type="email" required autoComplete="username" /></AutoField>
            <AutoField label="密码" name="password" required help="至少 8 个字符。已有账号请填写原密码。"><Input id="setup-password" name="password" type="password" required minLength={8} autoComplete="new-password" /></AutoField>
            <AutoField label="确认密码" className="block space-y-1 text-sm"><Input name="confirm_password" type="password" required minLength={8} autoComplete="new-password" /></AutoField>
            <Button type="submit" className="w-full" disabled={busy}>{busy && <Loader2 className="animate-spin" />}初始化并进入平台</Button>
            {signedIn ? <Button type="button" variant="link" className="w-full" disabled={busy} onClick={onReady}>稍后初始化，继续工作空间</Button>
              : setup.has_users && <Button type="button" variant="link" className="w-full" disabled={busy} onClick={() => { setStep("sign-in"); setError(""); }}>使用已有账号登录</Button>}
          </AutoForm>
        )}

        {setup && step === "sign-in" && (
          <AutoForm
            className="space-y-3"
            columns={1}
            fields={{
              email: { name: "邮箱", type: "email", required: true, config: { autoComplete: "username", placeholder: "邮箱" } },
              password: { name: "密码", type: "password", required: true, config: { autoComplete: "current-password", placeholder: "密码" } },
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
              {busy && <Loader2 className="animate-spin" />}登录
            </Button>
            {setup.registration_enabled && <Button
              type="button"
              variant="link"
              className="w-full"
              onClick={() => (setStep("register"), setError(""))}
            >
              还没有账号？注册
            </Button>}
            {setup.required && <Button type="button" variant="link" className="w-full" onClick={() => setStep("setup")}>初始化平台超级管理员</Button>}
          </AutoForm>
        )}

        {step === "register" && (
          <AutoForm
            className="space-y-3"
            columns={1}
            fields={{
              name: { name: "姓名", type: "text", required: true, config: { autoComplete: "name", placeholder: "姓名" } },
              email: { name: "邮箱", type: "email", required: true, config: { autoComplete: "username", placeholder: "邮箱" } },
              password: { name: "密码", type: "password", required: true, help: "至少 8 个字符。", config: { minLength: 8, autoComplete: "new-password", placeholder: "密码（至少 8 个字符）" } },
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
              {busy && <Loader2 className="animate-spin" />}注册
            </Button>
            <Button
              type="button"
              variant="link"
              className="w-full"
              onClick={() => (setStep("sign-in"), setError(""))}
            >
              已有账号？登录
            </Button>
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
              <p className="text-sm font-medium">创建工作空间</p>
              <AutoField label="名称" name="name" required><Input name="name" placeholder="名称，例如 运维平台" required /></AutoField>
              <AutoField label="标识" name="slug" required help="小写字母、数字和中划线。"><Input
                name="slug"
                placeholder="标识，例如 ops（小写字母、数字、中划线）"
                required
                pattern="[a-z0-9][a-z0-9-]{0,38}[a-z0-9]"
              /></AutoField>
              <Button type="submit" className="w-full" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}创建
              </Button>
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
              <p className="text-sm font-medium">加入工作空间</p>
              <AutoField label="邀请码" name="code" required><Input name="code" placeholder="邀请码" required className="uppercase" /></AutoField>
              <Button type="submit" variant="outline" className="w-full" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}加入
              </Button>
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
