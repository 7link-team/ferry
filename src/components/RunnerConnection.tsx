import { useEffect, useId, useRef, useState } from "react";
import { Dialog } from "radix-ui";
import { CheckCircle2, Copy, Loader2, Terminal, X } from "lucide-react";
import { api, errorText } from "../api";
import { href } from "../router";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs";

type Platform = "linux" | "macos" | "windows";
type Token = { id: string; token: string; expires_at: number; server: string };
type TokenStatus = { id: string; expires_at: number; used_at: number; host_id: string; state: "pending" | "expired" | "connected"; host_state: "online" | "offline" | "" };
type Props = { workspace: string; hostId?: string; onConnected?: () => void; inline?: boolean; autoGenerate?: boolean };

export function runnerServerOrigin(value: string): string {
  const raw = value.trim();
  let url: URL;
  try { url = new URL(raw); } catch { throw new Error("请输入完整的 HTTP 或 HTTPS 服务地址，仅包含域名或 IP 和端口。"); }
  if (!/^https?:\/\/[^/?#]+\/?$/i.test(raw) || /[\\\s]/.test(raw) || !["http:", "https:"].includes(url.protocol) || url.username || url.password || url.pathname !== "/" || url.search || url.hash || raw.includes("?") || raw.includes("#")) {
    throw new Error("请输入完整的 HTTP 或 HTTPS 服务地址，不含路径、账号或查询参数。");
  }
  if (url.protocol === "http:" && !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) {
    throw new Error("远程安装请使用 HTTPS，HTTP 仅支持本机测试");
  }
  return url.origin;
}

export function runnerInstallCommand(platform: Platform, server: string, token: string): string {
  const origin = runnerServerOrigin(server);
  const sh = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
  const ps = (s: string) => `'${s.replaceAll("'", "''")}'`;
  if (platform === "windows") {
    return `& { $previousToken = $env:FERRY_TOKEN; $previousModulePath = $env:PSModulePath; try { $bootstrap = Invoke-WebRequest -UseBasicParsing -Uri ${ps(`${origin}/api/runners/install.ps1`)} -MaximumRedirection 0 -ErrorAction Stop; $payload = '& { ' + $bootstrap.Content + [Environment]::NewLine + ${ps(`} -Server ${ps(origin)}`)}; $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($payload)); $env:FERRY_TOKEN = ${ps(token)}; $env:PSModulePath = $null; & powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded; if ($LASTEXITCODE -ne 0) { throw '安装失败，请查看上方错误。' } } finally { $env:FERRY_TOKEN = $previousToken; $env:PSModulePath = $previousModulePath } }`;
  }
  return `(umask 077; dir=$(mktemp -d) || exit 1; trap 'rm -rf "$dir"' EXIT; export FERRY_TOKEN=${sh(token)}; curl -fsSL --proto-redir '=https' ${sh(`${origin}/api/runners/install.sh`)} -o "$dir/install.sh" && sh "$dir/install.sh" --server ${sh(origin)})`;
}

function ConnectionFlow({ workspace, hostId, onConnected, autoGenerate }: Props) {
  const [platform, setPlatform] = useState<Platform>("linux");
  const [server, setServer] = useState(() => window.location.origin);
  const [token, setToken] = useState<Token>();
  const [status, setStatus] = useState<TokenStatus>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [pollError, setPollError] = useState("");
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(Date.now);
  const createRequest = useRef<AbortController | null>(null);
  const callback = useRef(onConnected);
  callback.current = onConnected;
  const notified = useRef(false);
  const autoStarted = useRef(false);
  const serverId = useId();
  const path = `/workspaces/${encodeURIComponent(workspace)}/runner-tokens`;
  const connected = status?.state === "connected" && status.host_state === "online";
  const used = !!status?.used_at || status?.state === "connected";
  const remaining = token ? Math.max(0, Math.ceil((token.expires_at - now) / 1000)) : 0;
  const expired = !!token && (status?.state === "expired" || (!used && remaining === 0));
  let serverError = "";
  try { runnerServerOrigin(server); } catch (cause) { serverError = errorText(cause); }
  const command = token && !expired && !used && !serverError ? runnerInstallCommand(platform, server, token.token) : "";

  useEffect(() => () => createRequest.current?.abort(), []);
  useEffect(() => {
    if (!token || connected || status?.state === "expired") return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const clock = setInterval(() => setNow(Date.now()), 1000);
    const poll = async () => {
      try {
        const next = await api<TokenStatus>(`${path}/${encodeURIComponent(token.id)}`, { signal: controller.signal });
        if (controller.signal.aborted) return;
        setStatus(next); setPollError("");
        if (next.state === "connected" && next.host_state === "online") {
          if (!notified.current) { notified.current = true; callback.current?.(); }
          return;
        }
        if (next.state === "expired") return;
      } catch (cause) {
        if (controller.signal.aborted) return;
        setPollError(errorText(cause));
      }
      timer = setTimeout(poll, 3000);
    };
    void poll();
    return () => { controller.abort(); clearTimeout(timer); clearInterval(clock); };
  }, [token, path, connected, status?.state === "expired"]);

  const generate = async () => {
    if (busy) return;
    createRequest.current?.abort();
    const controller = new AbortController(); createRequest.current = controller;
    setBusy(true); setError(""); setPollError(""); setCopied(false); setToken(undefined); setStatus(undefined); notified.current = false;
    try {
      const value = await api<Token>(path, { method: "POST", body: JSON.stringify({ host_id: hostId ?? "", ttl_minutes: 15 }), signal: controller.signal });
      if (controller.signal.aborted) return;
      setToken(value); setNow(Date.now());
      // Use the server's advertised origin initially; retain an explicitly edited address.
      setServer(current => current === window.location.origin && value.server ? value.server : current);
    } catch (cause) { if (!controller.signal.aborted) setError(errorText(cause)); }
    finally { if (!controller.signal.aborted) setBusy(false); }
  };
  useEffect(() => {
    if (!autoGenerate || autoStarted.current) return;
    autoStarted.current = true;
    void generate();
  }, [autoGenerate]);
  const copy = async () => {
    try { await navigator.clipboard.writeText(command); setCopied(true); setError(""); }
    catch { setError("无法访问剪贴板，请选中下方命令手动复制。"); }
  };

  return <div className="space-y-5 text-sm">
    <p className="text-muted-foreground">在目标主机运行安装命令，自动安装系统服务并连接。支持开机启动和异常恢复。</p>
    {!connected && <>
      <div className="space-y-2">
        <p className="font-medium">1. 选择主机系统</p>
        <Tabs value={platform} onValueChange={value => { setPlatform(value as Platform); setCopied(false); }}>
          <TabsList aria-label="主机操作系统" className="w-full"><TabsTrigger value="linux">Linux</TabsTrigger><TabsTrigger value="macos">macOS</TabsTrigger><TabsTrigger value="windows">Windows</TabsTrigger></TabsList>
        </Tabs>
        <p className="text-xs text-muted-foreground">{platform === "windows" ? "支持 x64；请以管理员身份打开 PowerShell。" : "自动识别 x64 / ARM64；安装系统服务时可能需要输入 sudo 密码。"}</p>
      </div>
      <div className="space-y-2">
        <label className="font-medium" htmlFor={serverId}>2. 目标主机可访问的服务地址</label>
        <Input id={serverId} value={server} onChange={e => { setServer(e.target.value); setCopied(false); }} aria-invalid={!!serverError} aria-describedby={`${serverId}-help`} placeholder="https://server.example.com" />
        <p id={`${serverId}-help`} className={serverError ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>{serverError || "填写本服务的域名或 IP 和端口；远程主机不能使用本机 localhost 地址。"}</p>
      </div>
    </>}
    {error && <p role="alert" className="text-destructive">{error}</p>}
    {!token && <Button onClick={generate} disabled={busy || !!serverError}>{busy && <Loader2 className="motion-safe:animate-spin"/>}{busy ? "正在生成…" : "生成安装命令"}</Button>}
    {command && <div className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2"><p className="font-medium">3. 复制命令并在目标主机执行</p><Button size="sm" variant="outline" onClick={copy}><Copy/>{copied ? "已复制" : "复制命令"}</Button></div>
      <textarea aria-label="安装命令" readOnly value={command} spellCheck={false} className="min-h-36 w-full resize-y rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed focus-visible:outline-ring" onFocus={e => e.currentTarget.select()}/>
      <p className="text-xs text-muted-foreground">一次性命令，剩余 {Math.floor(remaining / 60).toString().padStart(2, "0")}:{(remaining % 60).toString().padStart(2, "0")}。命令包含注册凭据，请勿分享。</p>
    </div>}
    {token && <div role="status" className="rounded-md border bg-muted/30 p-4">
      <div className="flex items-center gap-2 font-medium">{connected ? <CheckCircle2 className="size-4"/> : !expired ? <Loader2 className="size-4 motion-safe:animate-spin"/> : null}{connected ? "主机已连接" : expired ? "命令已过期" : used ? "已注册，等待主机上线…" : "等待主机连接…"}</div>
      <p className="mt-1 text-xs text-muted-foreground">{connected ? "系统服务已连接，可在主机详情查看状态。" : expired ? "请重新生成安装命令，旧命令不再可用。" : used ? "注册凭据已使用，正在确认在线状态。" : "执行安装命令后，此处会自动显示连接结果。"}</p>
      {connected && status?.host_id && <a className="mt-3 inline-block underline underline-offset-4" href={href({workspace, group:"host", item:"hosts", mode:"view", id:status.host_id}, {})}>查看主机详情</a>}
      {expired && <Button className="mt-3" variant="outline" onClick={generate} disabled={busy || !!serverError}>重新生成命令</Button>}
    </div>}
    {pollError && <p role="alert" className="text-destructive">{pollError} 正在重试连接状态查询。</p>}
  </div>;
}

export function RunnerConnection(props: Props) {
  const [open, setOpen] = useState(false);
  const flow = <ConnectionFlow key={`${props.workspace}:${props.hostId ?? ""}`} {...props}/>;
  if (props.inline) return flow;
  return <Dialog.Root open={open} onOpenChange={setOpen}>
    <Dialog.Trigger asChild><Button variant="outline" size="sm"><Terminal/>连接主机</Button></Dialog.Trigger>
    <Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40"/>
      <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(42rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-lg border bg-background p-6 shadow-lg">
        <div className="mb-2 flex items-center justify-between"><Dialog.Title className="text-lg font-semibold">连接主机</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label="关闭"><X/></Button></Dialog.Close></div>
        <Dialog.Description className="mb-5 text-sm text-muted-foreground">选择系统、复制命令，等待主机上线。</Dialog.Description>
        {open && flow}
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}
