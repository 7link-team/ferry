import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { CheckCircle2,Copy,HelpCircle,Loader2,RotateCcw,Terminal,X } from "lucide-react";
import { Dialog } from "radix-ui";
import { useEffect,useId,useRef,useState } from "react";
import { useTranslation } from "react-i18next";
import { api,errorText } from "../../shared/api/api";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { Input } from "../../shared/ui/input";
import { Tabs,TabsList,TabsTrigger } from "../../shared/ui/tabs";

type Platform = "linux" | "macos" | "windows";
/** Where the agent runs: the machine service account, or the installing user's own login. */
export type InstallScope = "system" | "user";
type Token = { id: string; token: string; expires_at: number; server: string };
type TokenStatus = { id: string; expires_at: number; used_at: number; host_id: string; state: "pending" | "expired" | "connected"; host_state: "online" | "offline" | "" };
type Props = { navigation: Navigation; workspace: string; hostId?: string; onConnected?: () => void; inline?: boolean; autoGenerate?: boolean };

export function runnerServerOrigin(value: string): string {
  const raw = value.trim();
  let url: URL;
  try { url = new URL(raw); } catch { throw new Error(commonText("resource-domain:runner.originRequired")); }
  if (!/^https?:\/\/[^/?#]+\/?$/i.test(raw) || /[\\\s]/.test(raw) || !["http:", "https:"].includes(url.protocol) || url.username || url.password || url.pathname !== "/" || url.search || url.hash || raw.includes("?") || raw.includes("#")) {
    throw new Error(commonText("resource-domain:runner.originOnly"));
  }
  if (url.protocol === "http:" && !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) {
    throw new Error(commonText("resource-domain:runner.httpsRequired"));
  }
  return url.origin;
}

const SCOPE_HELP: Record<InstallScope, Record<"windows" | "unix", string>> = {
  system: {
    windows: "resource-domain:runner.systemWindowsHelp",
    unix: "resource-domain:runner.systemUnixHelp",
  },
  user: {
    windows: "resource-domain:runner.userWindowsHelp",
    unix: "resource-domain:runner.userUnixHelp",
  },
};

function InstallScopeChoice({ platform, scope, onChange, wide = false }: { platform: Platform; scope: InstallScope; onChange: (scope: InstallScope) => void; wide?: boolean }) {
  useTranslation("resource-domain");
  return <div className="space-y-2">
    <Tabs value={scope} onValueChange={value => onChange(value as InstallScope)}>
      <TabsList aria-label={commonText("resource-domain:runner.installScope")} className={wide ? "w-full" : undefined}><TabsTrigger value="system">{commonText("resource-domain:runner.systemService")}</TabsTrigger><TabsTrigger value="user">{commonText("resource-domain:runner.currentUser")}</TabsTrigger></TabsList>
    </Tabs>
    <p className="text-xs text-muted-foreground">{commonText(SCOPE_HELP[scope][platform === "windows" ? "windows" : "unix"])}</p>
  </div>;
}

/**
 * One-click removal matching the install command: the native registration and
 * the default installation directory (program, connection credentials, host
 * key and logs). The CLI `service uninstall` alone keeps that data.
 */
export function runnerUninstallCommand(platform: Platform, scope: InstallScope = "system"): string {
  const name = "runner-agent";
  const ps = (text: string) => `'${text.replaceAll("'", "''")}'`;
  const message = (key: string) => ps(commonText(`resource-domain:runner.command.${key}`));
  if (platform !== "windows") {
    const root = scope === "system" ? `/var/lib/${name}` : platform === "macos" ? `$HOME/Library/Application Support/${name}` : `$HOME/.local/state/${name}`;
    const body = `d="${root}"; if [ -x "$d/runner-service" ]; then "$d/runner-service" service uninstall --name ${name}${scope === "user" ? " --user" : ""} || exit 1; fi; rm -rf "$d" && [ ! -e "$d" ] && echo "${commonText("resource-domain:runner.command.uninstalled").replace(/[\\$`"]/g, "\\$&")}"`;
    return scope === "system" ? `sudo sh -c '${body.replaceAll("'", "'\"'\"'")}'` : `(${body})`;
  }
  // The directly pasted user command must not `exit` the user's own shell.
  const steps = (root: string, user: boolean) => `$root = Join-Path ${root} '${name}'
  $binary = Join-Path $root 'runner-service.exe'
  if (Test-Path -LiteralPath $binary) {
    Write-Host ${message("removingRegistration")}
    & $binary service uninstall --name ${name}${user ? " --user" : ""}
    if ($LASTEXITCODE -ne 0) { throw ${ps(commonText("resource-domain:runner.command.uninstallExit", { code: "$LASTEXITCODE" })).replace(/\$LASTEXITCODE/, "' + $LASTEXITCODE + '")} }
  }${user ? "" : `
  # SCM deletes the registration only after the running service has stopped.
  $deadline = [DateTime]::UtcNow.AddSeconds(60)
  while (Get-Service -Name '${name}' -ErrorAction SilentlyContinue) {
    if ([DateTime]::UtcNow -gt $deadline) { throw ${message("stopTimeout")} }
    Start-Sleep -Milliseconds 500
  }`}
  for ($attempt = 0; Test-Path -LiteralPath $root; $attempt++) {
    try { Remove-Item -LiteralPath $root -Recurse -Force }
    catch { if ($attempt -ge 20) { throw }; Start-Sleep -Milliseconds 500 }
  }`;
  // Report the observable result in the window the user pasted into, whichever
  // process did the work, so success is never silent.
  const verify = (root: string, user: boolean) => `${user ? "" : `if (Get-Service -Name '${name}' -ErrorAction SilentlyContinue) { throw ${message("serviceRemains")} }
  `}if (Test-Path -LiteralPath (Join-Path ${root} '${name}')) { throw ${message("directoryRemains")} }
  Write-Host ${ps(commonText("resource-domain:runner.command.registrationRemoved", { registration: commonText(user ? "resource-domain:runner.scheduledTask" : "resource-domain:runner.systemService") }))} -ForegroundColor Green`;
  if (scope === "user") return `& {
  $ErrorActionPreference = 'Stop'
  Write-Host ${message("uninstallingUser")}
  ${steps("$env:LOCALAPPDATA", true)}
  ${verify("$env:LOCALAPPDATA", true)}
}`;
  const payload = (pause: boolean) => `$ErrorActionPreference = 'Stop'
try {
  ${steps("$env:ProgramData", false)}
} catch {
  [Console]::Error.WriteLine($_.ToString())
  ${pause ? `Read-Host ${message("uninstallClose")}\n  ` : ""}exit 1
}`;
  const encode = (text: string) => btoa(String.fromCharCode(...new Uint8Array(new Uint16Array([...text].flatMap(c => { const code = c.codePointAt(0)!; return code > 0xffff ? [0xd800 + ((code - 0x10000) >> 10), 0xdc00 + ((code - 0x10000) & 0x3ff)] : [code]; })).buffer)));
  return `& {
  $ErrorActionPreference = 'Stop'
  $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
  if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand ${encode(payload(false))}
    if ($LASTEXITCODE -ne 0) { throw ${message("uninstallFailed")} }
  } else {
    Write-Host ${message("requestUninstallAdmin")}
    try { $process = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', '${encode(payload(true))}') }
    catch { throw ${message("uninstallDenied")} }
    if ($process.ExitCode -ne 0) { throw ${message("uninstallAdminFailed")} }
  }
  ${verify("$env:ProgramData", false)}
}`;
}

function UninstallCommand({ platform, scope }: { platform: Platform; scope: InstallScope }) {
  useTranslation("resource-domain");
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState("");
  const command = runnerUninstallCommand(platform, scope);
  useEffect(() => setCopied(false), [command]);
  return <div className="space-y-2">
    <div className="flex flex-wrap items-center justify-between gap-2">
      <p className="font-medium">{commonText("resource-domain:runner.uninstallHeading")}</p>
      <Button type="button" size="sm" variant="outline" onClick={async () => { try { await navigator.clipboard.writeText(command); setCopied(true); setError(""); } catch { setError(commonText("resource-domain:runner.clipboardUnavailable")); } }}><Copy className="size-4"/>{copied ? commonText("resource-domain:runner.copied") : commonText("resource-domain:runner.copyUninstall")}</Button>
    </div>
    <textarea aria-label={commonText("resource-domain:runner.uninstallCommand")} readOnly value={command} spellCheck={false} className="min-h-24 w-full resize-y rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed focus-visible:outline-ring" onFocus={e => e.currentTarget.select()}/>
    <p className="text-xs text-muted-foreground">{scope === "system" ? (platform === "windows" ? commonText("resource-domain:runner.adminRequired") : commonText("resource-domain:runner.sudoRequired")) : commonText("resource-domain:runner.noAdminRequired")}{commonText("resource-domain:runner.localPrefix")}{platform === "windows" ? " PowerShell " : commonText("resource-domain:runner.terminal")}{commonText("resource-domain:runner.localSuffix")}</p>
    {error && <p role="alert" className="text-destructive">{error}</p>}
  </div>;
}

export function runnerInstallCommand(platform: Platform, server: string, token: string, scope: InstallScope = "system"): string {
  const origin = runnerServerOrigin(server);
  const sh = (s: string) => `'${s.replaceAll("'", "'\"'\"'")}'`;
  const ps = (s: string) => `'${s.replaceAll("'", "''")}'`;
  // The child PowerShell fetches and runs the installer itself: passing the
  // whole script through -EncodedCommand exceeds the Windows command line.
  const child = (user: boolean) => ps(`$ErrorActionPreference = 'Stop'
$bootstrap = Invoke-WebRequest -UseBasicParsing -Uri ${ps(`${origin}/api/runners/install.ps1`)} -MaximumRedirection 0 -ErrorAction Stop
& ([ScriptBlock]::Create($bootstrap.Content)) -Server ${ps(origin)}${user ? " -User" : ""}`);
  if (platform === "windows" && scope === "user") {
    return `& {
  $ErrorActionPreference = 'Stop'
  $previousToken = $env:FERRY_TOKEN
  $previousModulePath = $env:PSModulePath
  try {
    Write-Host ${ps(commonText("resource-domain:runner.command.fetchingInstaller"))}
    $payload = ${child(true)}
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($payload))
    $env:FERRY_TOKEN = ${ps(token)}
    $env:PSModulePath = $null
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded
    if ($LASTEXITCODE -ne 0) { throw ${ps(commonText("resource-domain:runner.command.installFailed"))} }
  } finally { $env:FERRY_TOKEN = $previousToken; $env:PSModulePath = $previousModulePath }
}`;
  }
  if (platform === "windows") {
    return `& {
  $ErrorActionPreference = 'Stop'
  $previousToken = $env:FERRY_TOKEN
  $previousModulePath = $env:PSModulePath
  try {
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
      Write-Host ${ps(commonText("resource-domain:runner.command.fetchingInstaller"))}
      $payload = ${child(false)}
      $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($payload))
      $env:FERRY_TOKEN = ${ps(token)}
      $env:PSModulePath = $null
      & powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand $encoded
      if ($LASTEXITCODE -ne 0) { throw ${ps(commonText("resource-domain:runner.command.installFailed"))} }
    } else {
      $pipeName = 'ferry-runner-' + [Guid]::NewGuid().ToString('N')
      $security = [IO.Pipes.PipeSecurity]::new()
      $security.AddAccessRule([IO.Pipes.PipeAccessRule]::new($principal.Identity.User, 'ReadWrite', 'Allow'))
      $security.AddAccessRule([IO.Pipes.PipeAccessRule]::new([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'), 'ReadWrite', 'Allow'))
      $pipe = [IO.Pipes.NamedPipeServerStream]::new($pipeName, 'Out', 1, 'Byte', 'Asynchronous', 0, 0, $security)
      try {
        $payload = @'
$ErrorActionPreference = 'Stop'
$pipe = [IO.Pipes.NamedPipeClientStream]::new('.', '__PIPE__', [IO.Pipes.PipeDirection]::In)
try {
  Write-Host ${ps(commonText("resource-domain:runner.command.waitingCredentials"))}
  $pipe.Connect(120000)
  $reader = [IO.StreamReader]::new($pipe)
  $env:FERRY_TOKEN = $reader.ReadLine()
  $env:PSModulePath = $null
  Write-Host ${ps(commonText("resource-domain:runner.command.fetchingInstaller"))}
  $bootstrap = Invoke-WebRequest -UseBasicParsing -Uri '__SERVER__/api/runners/install.ps1' -MaximumRedirection 0 -ErrorAction Stop
  & ([ScriptBlock]::Create($bootstrap.Content)) -Server '__SERVER__'
} catch {
  [Console]::Error.WriteLine($_.ToString())
  Read-Host ${ps(commonText("resource-domain:runner.command.installClose"))}
  exit 1
} finally { $env:FERRY_TOKEN = $null; $pipe.Dispose() }
'@.Replace('__PIPE__', $pipeName).Replace('__SERVER__', ${ps(origin)})
        $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($payload))
        $process = Start-Process powershell.exe -Verb RunAs -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encoded) -PassThru
        try {
          $wait = $pipe.WaitForConnectionAsync()
          if (-not $wait.Wait(120000)) { throw ${ps(commonText("resource-domain:runner.command.adminConnectTimeout"))} }
          $wait.GetAwaiter().GetResult()
          $writer = [IO.StreamWriter]::new($pipe)
          $writer.AutoFlush = $true
          $writer.WriteLine(${ps(token)})
          $process.WaitForExit()
          if ($process.ExitCode -ne 0) { throw ${ps(commonText("resource-domain:runner.command.installAdminFailed"))} }
        } finally { $process.Dispose() }
      } finally { $pipe.Dispose() }
    }
  } finally { $env:FERRY_TOKEN = $previousToken; $env:PSModulePath = $previousModulePath }
}`;
  }
  return `(umask 077; dir=$(mktemp -d) || exit 1; trap 'rm -rf "$dir"' EXIT; export FERRY_TOKEN=${sh(token)}; curl -fsSL --proto-redir '=https' ${sh(`${origin}/api/runners/install.sh`)} -o "$dir/install.sh" && sh "$dir/install.sh" --server ${sh(origin)}${scope === "user" ? " --user" : ""})`;
}

function ConnectionFlow({ navigation, workspace, hostId, onConnected, autoGenerate }: Props) {
  useTranslation("resource-domain");
  const [platform, setPlatform] = useState<Platform>("linux");
  const [scope, setScope] = useState<InstallScope>("system");
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
  const command = token && !expired && !used && !serverError ? runnerInstallCommand(platform, server, token.token, scope) : "";

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
    catch { setError(commonText("resource-domain:runner.clipboardUnavailable")); }
  };

  return <div className="space-y-5 text-sm">
    <p className="text-muted-foreground">{commonText("resource-domain:runner.installHelp")}</p>
    {!connected && <>
      <div className="space-y-2">
        <p className="font-medium">{commonText("resource-domain:runner.stepSystem")}</p>
        <Tabs value={platform} onValueChange={value => { setPlatform(value as Platform); setCopied(false); }}>
          <TabsList aria-label={commonText("resource-domain:runner.hostSystem")} className="w-full"><TabsTrigger value="linux">Linux</TabsTrigger><TabsTrigger value="macos">macOS</TabsTrigger><TabsTrigger value="windows">Windows</TabsTrigger></TabsList>
        </Tabs>
        <p className="text-xs text-muted-foreground">{platform === "windows" ? commonText("resource-domain:runner.windowsHelp") : commonText("resource-domain:runner.archHelp")}</p>
      </div>
      <div className="space-y-2">
        <p className="font-medium">{commonText("resource-domain:runner.stepScope")}</p>
        <InstallScopeChoice wide platform={platform} scope={scope} onChange={value => { setScope(value); setCopied(false); }}/>
      </div>
      <AutoField label={commonText("resource-domain:runner.stepAddress")} help={commonText("resource-domain:runner.serverHelp")} error={serverError || undefined}>
        <Input id={serverId} value={server} onChange={e => { setServer(e.target.value); setCopied(false); }} aria-invalid={!!serverError} aria-describedby={`${serverId}-help`} placeholder="https://server.example.com" />
      </AutoField>
    </>}
    {error && <p role="alert" className="text-destructive">{error}</p>}
    {!token && <Button onClick={generate} disabled={busy || !!serverError}>{busy && <Loader2 className="motion-safe:animate-spin"/>}{busy ? commonText("resource-domain:runner.generating") : commonText("resource-domain:runner.generate")}</Button>}
    {command && <div className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2"><p className="font-medium">{commonText("resource-domain:runner.stepExecute")}</p><Button size="sm" variant="outline" onClick={copy}><Copy/>{copied ? commonText("resource-domain:runner.copied") : commonText("resource-domain:runner.copyCommand")}</Button></div>
      <textarea aria-label={commonText("resource-domain:runner.installCommand")} readOnly value={command} spellCheck={false} className="min-h-36 w-full resize-y rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed focus-visible:outline-ring" onFocus={e => e.currentTarget.select()}/>
      <p className="text-xs text-muted-foreground">{commonText("resource-domain:runner.remaining", { time: `${Math.floor(remaining / 60).toString().padStart(2, "0")}:${(remaining % 60).toString().padStart(2, "0")}` })}</p>
    </div>}
    {command && <UninstallCommand platform={platform} scope={scope}/>}
    {token && <div role="status" className="rounded-md border bg-muted/30 p-4">
      <div className="flex items-center gap-2 font-medium">{connected ? <CheckCircle2 className="size-4"/> : !expired ? <Loader2 className="size-4 motion-safe:animate-spin"/> : null}{connected ? commonText("resource-domain:runner.hostConnected") : expired ? commonText("resource-domain:runner.commandExpired") : used ? commonText("resource-domain:runner.waitOnline") : commonText("resource-domain:runner.waitHost")}</div>
      <p className="mt-1 text-xs text-muted-foreground">{connected ? commonText("resource-domain:runner.connectedHelp") : expired ? commonText("resource-domain:runner.expiredHelp") : used ? commonText("resource-domain:runner.usedHelp") : commonText("resource-domain:runner.pendingHelp")}</p>
      {connected && status?.host_id && <a className="mt-3 inline-block underline underline-offset-4" href={navigation.href({workspace, group:"host", item:"hosts", mode:"view", id:status.host_id}, {})}>{commonText("resource-domain:runner.hostDetails")}</a>}
      {expired && <Button className="mt-3" variant="outline" onClick={generate} disabled={busy || !!serverError}>{commonText("resource-domain:runner.regenerate")}</Button>}
    </div>}
    {pollError && <p role="alert" className="text-destructive">{pollError} {commonText("resource-domain:runner.pollRetry")}</p>}
  </div>;
}

export function RunnerConnection(props: Props) {
  useTranslation("resource-domain");
  const [open, setOpen] = useState(false);
  const flow = <ConnectionFlow key={`${props.workspace}:${props.hostId ?? ""}`} {...props}/>;
  if (props.inline) return flow;
  return <Dialog.Root open={open} onOpenChange={setOpen}>
    <Dialog.Trigger asChild><Button variant="outline" size="sm"><Terminal/>{commonText("resource-domain:runner.connectHost")}</Button></Dialog.Trigger>
    <Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40"/>
      <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[min(42rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-y-auto rounded-lg border bg-background p-6 shadow-lg">
        <div className="mb-2 flex items-center justify-between"><Dialog.Title className="text-lg font-semibold">{commonText("resource-domain:runner.connectHost")}</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label={commonText("common-domain:common.close")}><X/></Button></Dialog.Close></div>
        <Dialog.Description className="mb-5 text-sm text-muted-foreground">{commonText("resource-domain:runner.connectHelp")}</Dialog.Description>
        {open && flow}
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

type RunnerUpdate = {
  available?: boolean;
  agent_version: string;
  target_version: string;
  supported: boolean;
  ready: boolean;
  enabled: boolean;
  auto_update: "inherit" | "on" | "off";
  global_auto_update: boolean;
  update: null | { state: string; version: string; message: string; auto?: { attempts: number; next_retry_at: number } };
};

export function HostAgentVersion({ workspace, hostId, version, available, updateState, online, canUpdate }: {
  workspace: string; hostId: string; version?: string | undefined; available?: boolean | undefined;
  updateState?: string | undefined; online: boolean; canUpdate: boolean;
}) {
  const [pending, setPending] = useState(false);
  const [accepted, setAccepted] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { setAccepted(false); setError(""); }, [workspace, hostId, version, updateState]);
  const busy = accepted || ["draining", "updating", "unknown"].includes(updateState ?? "");
  const update = async () => {
    setPending(true); setError("");
    try {
      const path = `/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/runner-updates`;
      const status = await api<RunnerUpdate>(path);
      if (!status.supported || !status.ready) {
        setError(commonText(`resource-domain:runner.${status.supported ? "waitHealthy" : "unsupportedHelp"}`));
        return;
      }
      await api(path, { method: "POST", body: "{}" });
      setAccepted(true);
    } catch (cause) { setError(errorText(cause)); }
    finally { setPending(false); }
  };
  return <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
    <span className="tabular-nums text-muted-foreground">HostAgent {version || "—"}</span>
    {busy ? <span role="status" className="text-primary">{commonText(`resource-domain:runner.${updateState === "unknown" ? "verifyReconnect" : updateState === "draining" || accepted ? "draining" : "updating"}`)}</span>
      : available && canUpdate && <Button type="button" variant="outline" size="xs" disabled={pending || !online} title={commonText("resource-domain:runner.updateWaitHelp")} onClick={event => { event.stopPropagation(); void update(); }}>{pending && <Loader2 className="size-3 animate-spin" />}{commonText("resource-domain:runner.updateNow")}</Button>}
    {error && <span role="alert" className="basis-full text-destructive">{error}</span>}
  </div>;
}

export function RunnerUpdateStatus({ workspace, hostId, canUpdate = false }: { workspace: string; hostId: string; canUpdate?: boolean }) {
  useTranslation("resource-domain");
  const [status, setStatus] = useState<RunnerUpdate>();
  const [error, setError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    setStatus(undefined); setError("");
    const refresh = () => {
      void api<RunnerUpdate>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/runner-updates`, { signal: controller.signal })
        .then(value => { if (!controller.signal.aborted) { setStatus(value); setError(""); } })
        .catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
    };
    refresh();
    const timer = window.setInterval(refresh, 5000);
    return () => { controller.abort(); window.clearInterval(timer); };
  }, [workspace, hostId]);
  const labels: Record<string, string> = { draining: commonText("resource-domain:runner.draining"), updating: commonText("resource-domain:runner.updating"), unknown: commonText("resource-domain:runner.verifyReconnect"), succeeded: commonText("resource-domain:runner.updated"), rolled_back: commonText("resource-domain:runner.rolledBack"), failed: commonText("resource-domain:runner.updateFailed"), retrying: commonText("resource-domain:runner.retrying"), cancelled: commonText("common-domain:dashboard.cancelled") };
  return <section aria-label={commonText("resource-domain:runner.updateStatus")} className="space-y-2 rounded-xl border bg-card p-5 text-sm">
    <h2 className="font-semibold">{commonText("resource-domain:runner.updateTitle")}</h2>
    {error ? <p role="alert" className="text-destructive">{commonText("resource-domain:runner.refreshFailed")}{error}</p> : !status ? <LoadingSkeleton variant="inline" label={commonText("resource-domain:runner.loadingUpdate")} /> : null}
    {status && <>
      <HostAgentVersion workspace={workspace} hostId={hostId} version={status.agent_version} available={status.available && status.supported} updateState={status.update?.state} online={status.ready} canUpdate={canUpdate} />
      <p className="text-xs text-muted-foreground">{commonText("resource-domain:runner.versions", { current: status.agent_version || "—", server: status.target_version })}</p>
      <p>{commonText("resource-domain:runner.autoPolicy")}{status.enabled ? commonText("resource-domain:status.enabled") : commonText("resource-domain:status.off")}{status.auto_update === "inherit" ? commonText("resource-domain:runner.inherited") : commonText("resource-domain:runner.overridden")}</p>
      {status.update && <div role="status" aria-live="polite"><p>{labels[status.update.state] ?? status.update.state} · {status.update.version}</p><p className="text-muted-foreground">{status.update.message}</p>
        {!!status.update.auto?.next_retry_at && <p>{commonText("resource-domain:runner.nextRetry")}{new Date(status.update.auto.next_retry_at).toLocaleString(currentLocale())}</p>}
      </div>}
      {!status.ready && <p className="text-muted-foreground">{commonText("resource-domain:runner.waitHealthy")}</p>}
      {!status.supported && <p className="text-muted-foreground">{commonText("resource-domain:runner.unsupportedHelp")}</p>}
    </>}
  </section>;
}

export function HostConnectionCommand({ workspace, hostId, compact = false }: { workspace: string; hostId: string; compact?: boolean }) {
  useTranslation("resource-domain");
  const [platform, setPlatform] = useState<Platform>("linux");
  const [scope, setScope] = useState<InstallScope>("system");
  const [server, setServer] = useState(window.location.origin);
  const [token, setToken] = useState("");
  const [state, setState] = useState("");
  const [busy, setBusy] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const path = `/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}/connection-credentials`;
  useEffect(() => {
    const controller = new AbortController();
    void api<{ token: string; server: string; state: string }>(path, { signal: controller.signal })
      .then(value => { if (!controller.signal.aborted) { setToken(value.token); setServer(value.server); setState(value.state); } })
      .catch(cause => { if (!controller.signal.aborted) setError(errorText(cause)); });
    return () => controller.abort();
  }, [path]);
  useEffect(() => {
    const timer = setInterval(() => {
      void api<{ state: string }>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}`)
        .then(value => setState(value.state)).catch(() => {});
    }, 5000);
    return () => clearInterval(timer);
  }, [workspace, hostId]);
  let command = "";
  let serverError = "";
  if (token) try { command = runnerInstallCommand(platform, server, token, scope); }
    catch (cause) { serverError = errorText(cause); }
  const reset = async () => {
    setBusy(true); setError("");
    try {
      const value = await api<{ token: string; server: string; state: string }>(path, { method: "POST" });
      setToken(value.token); setState(value.state); setCopied(false);
    } catch (cause) { setError(errorText(cause)); throw cause; }
    finally { setBusy(false); }
  };
  return <section className={compact ? "min-w-0 space-y-4 rounded-xl border bg-card p-5 text-sm" : "mt-5 space-y-3 border-t pt-5 text-sm"} aria-label={commonText("resource-domain:runner.currentCommand")}>
    <div className="flex flex-wrap items-center justify-between gap-2"><h2 className="font-semibold">{commonText("resource-domain:runner.currentCommand")}</h2><span className={state === "online" ? "text-primary" : "text-muted-foreground"}>{state === "online" ? commonText("resource-domain:runner.connected") : state === "offline" ? commonText("resource-domain:runner.disconnected") : commonText("resource-domain:runner.waiting")}</span></div>
    <Tabs value={platform} onValueChange={value => { setPlatform(value as Platform); setCopied(false); }}><TabsList aria-label={commonText("resource-domain:runner.hostSystem")}><TabsTrigger value="linux">Linux</TabsTrigger><TabsTrigger value="macos">macOS</TabsTrigger><TabsTrigger value="windows">Windows</TabsTrigger></TabsList></Tabs>
    <InstallScopeChoice platform={platform} scope={scope} onChange={value => { setScope(value); setCopied(false); }}/>
    <AutoField label={<span>{commonText("resource-domain:runner.serverAddress")}</span>} className="block space-y-1"><Input value={server} onChange={event => { setServer(event.target.value); setCopied(false); }} aria-label={commonText("resource-domain:runner.serverAddress")} /></AutoField>
    {serverError && <p role="alert" className="text-destructive">{serverError}</p>}
    {error && <p role="alert" className="text-destructive">{error}</p>}
    <div className="flex items-center justify-between gap-2"><span className="text-muted-foreground">{commonText("resource-domain:runner.tokenHelp")}</span><Button type="button" size="sm" variant="outline" disabled={!command} onClick={async () => { try { await navigator.clipboard.writeText(command); setCopied(true); } catch { setError(commonText("resource-domain:runner.copyFailed")); } }}><Copy className="size-4" />{copied ? commonText("resource-domain:runner.copied") : commonText("resource-domain:runner.copyCommand")}</Button></div>
    <textarea aria-label={commonText("resource-domain:runner.currentCommand")} readOnly spellCheck={false} value={command} className="min-h-36 w-full resize-y rounded-md border bg-muted/40 p-3 font-mono text-xs leading-relaxed" onFocus={event => event.currentTarget.select()} />
    <UninstallCommand platform={platform} scope={scope}/>
    <div className="flex items-center justify-end gap-1"><span title={commonText("resource-domain:runner.resetHelp")} aria-label={commonText("resource-domain:runner.resetExplanation")}><HelpCircle className="size-4 text-muted-foreground" /></span><Button type="button" variant="outline" size="sm" disabled={busy || !token} onClick={() => setConfirmReset(true)}><RotateCcw className="size-4" />{commonText("resource-domain:runner.resetConnection")}</Button></div>
    {confirmReset && <DeleteConfirmDialog action={commonText("resource-domain:runner.reset")} target={commonText("resource-domain:runner.currentAgent")} description={commonText("resource-domain:runner.resetConfirm")} onClose={() => setConfirmReset(false)} onConfirm={reset} />}
  </section>;
}
