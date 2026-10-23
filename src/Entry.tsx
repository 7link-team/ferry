/**
 * Register / sign in / create or join a workspace.
 *
 * Rendered in place of the shell, on whatever URL the visitor arrived at —
 * never by redirecting to a login route. That is what makes a shared deep
 * link work: sign in and the same URL renders its target page, with its
 * filters and scroll offset intact (§5.3).
 */
import { useState } from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, errorText } from "./api";
import { navigate } from "./router";

type Step = "sign-in" | "register" | "workspace";

export function Entry({ onReady, error: outerError }: { onReady: () => void; error: string }) {
  const [step, setStep] = useState<Step>("sign-in");
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

        {step === "sign-in" && (
          <form
            className="space-y-3"
            onSubmit={(event) => {
              event.preventDefault();
              const body = fields(event.currentTarget);
              void run(async () => {
                await api("/sessions", { method: "POST", body: JSON.stringify(body) });
                onReady();
              });
            }}
          >
            <Input name="email" type="email" placeholder="邮箱" required autoComplete="username" />
            <Input
              name="password"
              type="password"
              placeholder="密码"
              required
              autoComplete="current-password"
            />
            <Button type="submit" className="w-full" disabled={busy}>
              {busy && <Loader2 className="animate-spin" />}登录
            </Button>
            <Button
              type="button"
              variant="link"
              className="w-full"
              onClick={() => (setStep("register"), setError(""))}
            >
              还没有账号？注册
            </Button>
          </form>
        )}

        {step === "register" && (
          <form
            className="space-y-3"
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
            <Input name="name" placeholder="姓名" required autoComplete="name" />
            <Input name="email" type="email" placeholder="邮箱" required autoComplete="username" />
            <Input
              name="password"
              type="password"
              placeholder="密码（至少 10 个字符）"
              required
              minLength={10}
              autoComplete="new-password"
            />
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
          </form>
        )}

        {step === "workspace" && (
          <div className="space-y-6">
            <form
              className="space-y-3"
              onSubmit={(event) => {
                event.preventDefault();
                const body = fields(event.currentTarget);
                void run(async () => {
                  const created = await api<{ slug: string }>("/workspaces", {
                    method: "POST",
                    body: JSON.stringify(body),
                  });
                  enter(created.slug);
                  onReady();
                });
              }}
            >
              <p className="text-sm font-medium">创建工作空间</p>
              <Input name="name" placeholder="名称，例如 运维平台" required />
              <Input
                name="slug"
                placeholder="标识，例如 ops（小写字母、数字、中划线）"
                required
                pattern="[a-z0-9][a-z0-9-]{0,38}[a-z0-9]"
              />
              <Button type="submit" className="w-full" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}创建
              </Button>
            </form>

            <form
              className="space-y-3 border-t border-border pt-5"
              onSubmit={(event) => {
                event.preventDefault();
                const body = fields(event.currentTarget);
                void run(async () => {
                  const joined = await api<{ slug: string }>("/workspaces/actions/join", {
                    method: "POST",
                    body: JSON.stringify(body),
                  });
                  enter(joined.slug);
                  onReady();
                });
              }}
            >
              <p className="text-sm font-medium">加入工作空间</p>
              <Input name="code" placeholder="邀请码" required className="uppercase" />
              <Button type="submit" variant="outline" className="w-full" disabled={busy}>
                {busy && <Loader2 className="animate-spin" />}加入
              </Button>
            </form>
          </div>
        )}
      </div>
    </div>
  );
}

/** Only rewrites the workspace segment, so a deep link keeps its destination. */
function enter(slug: string) {
  const parts = location.pathname.replace(/^\/+/, "").split("/");
  if (parts.length > 1 && parts[1]) {
    parts[0] = slug;
    history.replaceState(history.state, "", `/${parts.join("/")}${location.search}`);
  } else {
    navigate({ workspace: slug, mode: "list" }, {}, { replace: true });
  }
}
