import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { HostConnectionCommand, RunnerConnection, runnerInstallCommand, runnerServerOrigin, runnerUninstallCommand } from "./RunnerConnection";

it.skipIf(process.platform !== "win32")("rejects legacy Runner help before native installation with an actionable error", () => {
  const harness = `
$ErrorActionPreference = 'Stop'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path (Get-Location) 'scripts/runner-install.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$assignment = $ast.Find({ param($node) $node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -eq '$installHelp' }, $true)
$guards = @($assignment.Parent.Statements | Where-Object { $_.Extent.StartOffset -gt $assignment.Extent.EndOffset } | Select-Object -First 2)
if ($guards.Count -ne 2 -or $guards[1].Extent.Text -notlike '*$User*') { throw 'Runner compatibility checks missing' }
$check = [ScriptBlock]::Create($assignment.Extent.Text + [Environment]::NewLine + ($guards.Extent.Text -join [Environment]::NewLine))
$User = $false
function FixtureRunner {
  if (($args -join ' ') -ne 'service install --help') { throw 'Test must never install a service' }
  $global:LASTEXITCODE = 0
  $global:fixtureHelp
}
$binary = 'FixtureRunner'; $lines = @('fixture-version')
$global:fixtureHelp = 'Usage: service install --server <SERVER> --name <NAME>'
$rejected = $false
try { & $check } catch {
  if ($_.Exception.Message -notlike '*Update the Ferry server*Nothing installed.*') { throw }
  $rejected = $true
}
if (-not $rejected) { throw 'Legacy Runner must be rejected' }
$global:fixtureHelp = '      --rebind'
& $check
# Released Runners before Windows current-user support documented --user as
# Linux/macOS only and refuse it at install time; reject them before installing.
$User = $true
$global:fixtureHelp = "      --user    Install for the current user (Linux/macOS), rather than the machine." + [Environment]::NewLine + '      --rebind'
$rejected = $false
try { & $check } catch {
  if ($_.Exception.Message -notlike '*does not support current-user installation*Nothing installed.*') { throw }
  $rejected = $true
}
if (-not $rejected) { throw 'Runner without Windows current-user support must be rejected' }
$global:fixtureHelp = "      --user    Install for the current user instead of the machine (Windows logon task, systemd --user or LaunchAgent)" + [Environment]::NewLine + '      --rebind'
& $check
Write-Output 'PASS legacy rejection and compatible CLI'
`;
  const output = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(harness, "utf16le").toString("base64")], { encoding: "utf8", windowsHide: true });
  expect(output).toContain("PASS legacy rejection and compatible CLI");
});

it("generates current-user commands without elevation, keeping system commands unchanged", () => {
  const sh = runnerInstallCommand("linux", "https://example.com", "t");
  expect(runnerInstallCommand("linux", "https://example.com", "t", "system")).toBe(sh);
  expect(sh).not.toContain("--user");
  expect(runnerInstallCommand("macos", "https://example.com", "t", "user")).toBe(sh.replace("--server 'https://example.com')", "--server 'https://example.com' --user)"));
  const ps = runnerInstallCommand("windows", "https://example.com", "t");
  expect(runnerInstallCommand("windows", "https://example.com", "t", "system")).toBe(ps);
  const user = runnerInstallCommand("windows", "https://example.com", "t", "user");
  expect(user).toContain("-Server ''https://example.com'' -User");
  for (const elevated of ["-Verb RunAs", "PipeSecurity", "IsInRole"]) expect(user).not.toContain(elevated);
  for (const shared of ["-MaximumRedirection 0", "-EncodedCommand $encoded", "$env:PSModulePath = $null", "$env:FERRY_TOKEN = $previousToken"]) expect(user).toContain(shared);
});

it("generates one-click uninstall commands for each platform and install scope", () => {
  expect(runnerUninstallCommand("linux")).toBe(`sudo sh -c 'd="/var/lib/runner-agent"; if [ -x "$d/runner-service" ]; then "$d/runner-service" service uninstall --name runner-agent || exit 1; fi; rm -rf "$d" && echo "Runner uninstalled."'`);
  expect(runnerUninstallCommand("macos", "system")).toBe(runnerUninstallCommand("linux", "system"));
  const linuxUser = runnerUninstallCommand("linux", "user");
  expect(linuxUser).toContain('d="$HOME/.local/state/runner-agent"');
  expect(linuxUser).toContain("service uninstall --name runner-agent --user");
  expect(linuxUser).not.toContain("sudo");
  expect(runnerUninstallCommand("macos", "user")).toContain('d="$HOME/Library/Application Support/runner-agent"');
  const windowsUser = runnerUninstallCommand("windows", "user");
  expect(windowsUser).toContain("Join-Path $env:LOCALAPPDATA 'runner-agent'");
  expect(windowsUser).toContain("service uninstall --name runner-agent --user");
  for (const elevated of ["RunAs", "exit 1", "Get-Service"]) expect(windowsUser).not.toContain(elevated);
  const windowsSystem = runnerUninstallCommand("windows", "system");
  expect(windowsSystem).toContain("-Verb RunAs -Wait -PassThru");
  const payloads = [...windowsSystem.matchAll(/-EncodedCommand'?,? '?([A-Za-z0-9+/=]{20,})/g)].map(m => new TextDecoder("utf-16le").decode(Uint8Array.from(atob(m[1]!), c => c.charCodeAt(0))));
  expect(payloads).toHaveLength(2);
  for (const payload of payloads) {
    expect(payload).toContain("Join-Path $env:ProgramData 'runner-agent'");
    expect(payload).toContain("service uninstall --name runner-agent\n");
    expect(payload).toContain("while (Get-Service -Name 'runner-agent'");
  }
  expect(payloads[0]).not.toContain("Read-Host");
  expect(payloads[1]).toContain("Read-Host");
});

it.skipIf(process.platform === "win32")("runs the current-user Unix uninstall against a fixture Runner and parses the sudo command", () => {
  const script = `
set -eu
export HOME=$(mktemp -d)
trap 'rm -rf "$HOME"' EXIT
sh -n -c ${JSON.stringify(runnerUninstallCommand("linux", "system").replace(/^sudo /, ""))}
root="$HOME/.local/state/runner-agent"
mkdir -p "$root/versions"
printf '#!/bin/sh\\nprintf "%%s " "$@" > "$HOME/args"\\nexit "\${FIXTURE_FAIL:-0}"\\n' > "$root/runner-service"
chmod 700 "$root/runner-service"
if FIXTURE_FAIL=3 sh -c ${JSON.stringify(runnerUninstallCommand("linux", "user"))}; then echo 'failure must be reported'; exit 1; fi
[ -x "$root/runner-service" ] || { echo 'failed deregistration must keep files'; exit 1; }
sh -c ${JSON.stringify(runnerUninstallCommand("linux", "user"))}
[ "$(cat "$HOME/args")" = "service uninstall --name runner-agent --user " ] || { echo "wrong arguments: $(cat "$HOME/args")"; exit 1; }
[ ! -e "$root" ] || { echo 'installation directory must be removed'; exit 1; }
sh -c ${JSON.stringify(runnerUninstallCommand("linux", "user"))}
echo 'unix uninstall verified'
`;
  expect(execFileSync("sh", ["-c", script], { encoding: "utf8" })).toContain("unix uninstall verified");
});

it.skipIf(process.platform !== "win32")("parses the elevated uninstall payload and runs the current-user uninstall against a fixture Runner", () => {
  const system = runnerUninstallCommand("windows", "system");
  const payloads = [...system.matchAll(/-EncodedCommand'?,? '?([A-Za-z0-9+/=]{20,})/g)].map(m => m[1]!);
  const user = runnerUninstallCommand("windows", "user");
  const harness = `
$ErrorActionPreference = 'Stop'
$systemCommand = @'
${system}
'@
foreach ($text in @($systemCommand) + @(${payloads.map(p => `[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('${p}'))`).join(", ")})) {
  $errors = $null
  [void][Management.Automation.Language.Parser]::ParseInput($text, [ref]$null, [ref]$errors)
  if ($errors.Count) { throw ($errors | Out-String) }
}
$sandbox = Join-Path ([IO.Path]::GetTempPath()) ('runner-uninstall-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $sandbox
$fixture = Join-Path $sandbox 'fixture.exe'
Add-Type -OutputAssembly $fixture -OutputType ConsoleApplication -TypeDefinition 'public static class Fixture { public static int Main(string[] args) { System.IO.File.WriteAllText(System.Environment.GetEnvironmentVariable("FIXTURE_ARGS"), string.Join(" ", args)); return System.Environment.GetEnvironmentVariable("FIXTURE_FAIL") == "1" ? 3 : 0; } }'
$savedLocal = $env:LOCALAPPDATA
try {
  $env:LOCALAPPDATA = Join-Path $sandbox 'local'
  $env:FIXTURE_ARGS = Join-Path $sandbox 'args.txt'
  $root = Join-Path $env:LOCALAPPDATA 'runner-agent'
  # Installed: the registration is removed through the Runner, then all files.
  $null = New-Item -ItemType Directory -Path (Join-Path $root 'versions')
  Copy-Item $fixture (Join-Path $root 'runner-service.exe')
  Set-Content (Join-Path $root 'service.json') '{}'
  $env:FIXTURE_FAIL = '0'
  & { ${user} } | Out-Null
  if ((Get-Content $env:FIXTURE_ARGS) -ne 'service uninstall --name runner-agent --user') { throw 'Wrong uninstall arguments' }
  if (Test-Path -LiteralPath $root) { throw 'Installation directory must be removed' }
  # Not installed: succeeds without invoking anything.
  Remove-Item $env:FIXTURE_ARGS
  & { ${user} } | Out-Null
  if (Test-Path $env:FIXTURE_ARGS) { throw 'Nothing should run when not installed' }
  # A failed deregistration keeps the files, so it can be retried.
  $null = New-Item -ItemType Directory -Path $root
  Copy-Item $fixture (Join-Path $root 'runner-service.exe')
  $env:FIXTURE_FAIL = '1'
  $failed = $false
  try { & { ${user} } | Out-Null } catch { $failed = $true }
  if (-not $failed -or -not (Test-Path (Join-Path $root 'runner-service.exe'))) { throw 'Failure must be reported and keep the installation' }
} finally {
  $env:LOCALAPPDATA = $savedLocal
  Remove-Item -LiteralPath $sandbox -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Output 'uninstall commands verified'
`;
  // The embedded payloads exceed the Windows command-line limit for -EncodedCommand.
  const file = join(mkdtempSync(join(tmpdir(), "runner-uninstall-test-")), "harness.ps1");
  writeFileSync(file, "﻿" + harness, "utf8");
  try {
    const output = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", file], { encoding: "utf8", windowsHide: true });
    expect(output).toContain("uninstall commands verified");
  } finally { rmSync(dirname(file), { recursive: true, force: true }); }
});

it.skipIf(process.platform !== "win32")("runs the generated current-user Windows command without elevation and restores the environment", () => {
  const command = runnerInstallCommand("windows", "https://example.com", "fixture-token", "user");
  const harness = `
$ErrorActionPreference = 'Stop'
$global:nativePowerShell = (Get-Command powershell.exe).Source
function Invoke-WebRequest {
  param([switch]$UseBasicParsing, $Uri, $MaximumRedirection, $ErrorAction)
  if ($MaximumRedirection -ne 0) { throw 'Redirects must be disabled' }
  if ($global:scenario -eq 'download-failure') { throw 'fixture download failure' }
  return @{ Content = @'
param([string]$Server, [switch]$User)
if ($Server -ne 'https://example.com' -or -not $User) { throw 'Installer must receive -Server and -User' }
if ($env:FERRY_TOKEN -ne 'fixture-token') { throw 'Child token missing' }
if ($env:RUNNER_COMMAND_TEST_CASE -eq 'installer-failure') { exit 1 }
'@ }
}
function Start-Process { throw 'Current-user installation must not request elevation' }
function powershell.exe {
  if ($env:PSModulePath) { throw 'Child inherited PSModulePath' }
  $global:childCalls++
  & $global:nativePowerShell @args
  $global:LASTEXITCODE = $LASTEXITCODE
}
foreach ($case in @('success', 'installer-failure', 'download-failure')) {
  $env:FERRY_TOKEN = 'fixture-original'
  $global:scenario = $case
  $env:RUNNER_COMMAND_TEST_CASE = $case
  $global:childCalls = 0
  $failed = $false
  try { ${command} } catch { $failed = $true; if ($case -eq 'success') { throw } }
  if ($env:FERRY_TOKEN -ne 'fixture-original') { throw 'Token was not restored' }
  if ($failed -ne ($case -ne 'success')) { throw "Unexpected result: $case" }
  if ($global:childCalls -ne $(if ($case -eq 'download-failure') { 0 } else { 1 })) { throw 'Child invocation count mismatch' }
}
Write-Output 'current-user command verified'
`;
  const output = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(harness, "utf16le").toString("base64")], { encoding: "utf8", windowsHide: true, env: Object.fromEntries(Object.entries(process.env).filter(([key]) => key.toLowerCase() !== "psmodulepath")) });
  expect(output).toContain("current-user command verified");
});

it("accepts only HTTP(S) origins, without credentials, paths or query", () => {
  expect(runnerServerOrigin(" https://runner.example:8443/ ")).toBe("https://runner.example:8443");
  for (const value of ["file:///tmp", "https://user:pass@example.com", "https://example.com/api", "https://example.com/foo/..", "https://example.com/?x=1", "https://example.com/#x", "https://example.com\\evil"]) expect(() => runnerServerOrigin(value)).toThrow();
});
it("quotes tokens and prevents insecure redirects or Windows temporary script execution", () => {
  const token = "abc'$(touch /tmp/pwn);`whoami`";
  const sh = runnerInstallCommand("linux", "https://example.com", token);
  expect(sh).toContain("abc'\"'\"'$(touch /tmp/pwn);`whoami`");
  expect(sh).toContain("mktemp -d");
  expect(sh).toContain("--proto-redir '=https'");
  expect(sh).toMatch(/curl .*&& sh /);
  expect(sh).toContain("--server 'https://example.com'");
  const ps = runnerInstallCommand("windows", "https://example.com", token);
  expect(ps).toContain("abc''$(touch /tmp/pwn);`whoami`");
  expect(ps).not.toContain("-OutFile");
  expect(ps).not.toContain("GetTempPath");
  expect(ps).toContain("-MaximumRedirection 0");
  expect(ps).toContain("-EncodedCommand $encoded");
  expect(ps).toContain("[Text.Encoding]::Unicode.GetBytes($payload)");
  expect(ps).toContain("-ErrorAction Stop");
  expect(ps).toContain("-Server ''https://example.com''");
  expect(ps).toContain("$previousModulePath = $env:PSModulePath");
  expect(ps).toContain("$env:PSModulePath = $null");
  expect(ps).toContain("-Verb RunAs");
  expect(ps).toContain("PipeSecurity");
  expect(ps).toContain("$env:PSModulePath = $previousModulePath");
});
let root: Root, host: HTMLDivElement;
let status: Record<string, unknown>;
let requests: {url: string; init: RequestInit}[];
const click = async (label: string) => {
  const button = [...host.querySelectorAll("button")].find(b => b.textContent === label)!;
  expect(button).toBeDefined();
  await act(async () => button.click());
};
beforeEach(() => {
  vi.useFakeTimers();
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  status = {id:"t1", state:"pending", host_state:"", host_id:"", used_at:0};
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({url, init});
    return {ok:true, status:200, json:async () => init.method === "POST" ? {id:"t1", token:"secret", server:"https://example.com", expires_at: Date.now()+15000} : status};
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.unstubAllGlobals(); });
it("keeps the host command visible offline and replaces it only after confirmed reset", async () => {
  let online = true;
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => ({
    ok: true, status: 200,
    json: async () => url.endsWith("/connection-credentials")
      ? { token: init.method === "POST" ? "new-token" : "old-token", server: "https://example.com", state: init.method === "POST" ? "offline" : "online" }
      : { state: online ? "online" : "offline" },
  })));
  await act(async () => root.render(<HostConnectionCommand workspace="ops" hostId="h1" />));
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="当前连接命令"]')?.value).toContain("old-token");
  online = false;
  await act(async () => vi.advanceTimersByTimeAsync(5000));
  expect(host.textContent).toContain("已断开连接");
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="当前连接命令"]')?.value).toContain("old-token");
  await click("重置连接");
  expect(document.body.textContent).toContain("旧命令和令牌将失效");
  expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "POST")).toBe(false);
  await act(async () => [...document.body.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "确认重置")!.click());
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="当前连接命令"]')?.value).toContain("new-token");
});
it("creates a 15 minute scoped token, copies command, and reports online only once", async () => {
  const connected = vi.fn(), writeText = vi.fn(async () => {});
  Object.defineProperty(navigator,"clipboard",{configurable:true, value:{writeText}});
  await act(async () => root.render(<RunnerConnection workspace="ops" hostId="h1" inline onConnected={connected}/>));
  expect(requests).toHaveLength(0);
  await click("生成安装命令");
  expect(host.textContent).toContain("剩余 00:15");
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(host.textContent).toContain("剩余 00:14");
  expect(JSON.parse(requests[0]!.init.body as string)).toEqual({host_id:"h1",ttl_minutes:15});
  await click("复制命令"); expect(writeText).toHaveBeenCalledWith(expect.stringContaining("FERRY_TOKEN='secret'"));
  status = {id:"t1",state:"connected",host_state:"offline",host_id:"h1",used_at:Date.now()};
  await act(async () => vi.advanceTimersByTimeAsync(3000)); expect(connected).not.toHaveBeenCalled();
  status.host_state = "online";
  await act(async () => vi.advanceTimersByTimeAsync(3000)); expect(connected).toHaveBeenCalledTimes(1);
  expect(host.textContent).toContain("主机已连接"); expect(host.querySelector("a")?.href).toContain("/workspaces/ops/nodes/h1");
  const count = requests.length;
  await act(async () => vi.advanceTimersByTimeAsync(6000)); expect(requests).toHaveLength(count);
});
it("switches both connection surfaces between system service and current-user commands", async () => {
  const choose = async (label: string) => {
    const tab = [...host.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(t => t.textContent === label)!;
    expect(tab).toBeDefined();
    await act(async () => { tab.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })); });
  };
  await act(async () => root.render(<RunnerConnection workspace="ops" inline/>));
  await click("生成安装命令");
  const command = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="安装命令"]')!.value;
  expect(host.textContent).toContain("以系统服务运行，开机自动启动");
  expect(command()).not.toContain("--user");
  const uninstall = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="卸载命令"]')!.value;
  expect(uninstall()).toBe(runnerUninstallCommand("linux", "system"));
  await choose("当前用户");
  expect(command()).toContain("--server 'https://example.com' --user");
  expect(uninstall()).toBe(runnerUninstallCommand("linux", "user"));
  expect(host.textContent).toContain("无需 sudo");
  await choose("Windows");
  expect(command()).toContain("-User");
  expect(command()).not.toContain("RunAs");
  expect(host.textContent).toContain("仅在该账号登录 Windows 期间运行");
  await choose("系统服务");
  expect(command()).toContain("-Verb RunAs");
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.endsWith("/connection-credentials") ? { token: "host-token", server: "https://example.com", state: "online" } : { state: "online" } })));
  await act(async () => root.unmount()); root = createRoot(host);
  await act(async () => root.render(<HostConnectionCommand workspace="ops" hostId="h1" />));
  const current = () => host.querySelector<HTMLTextAreaElement>('textarea[aria-label="当前连接命令"]')!.value;
  expect(current()).not.toContain("--user");
  await choose("Windows");
  await choose("当前用户");
  expect(current()).toContain("-User");
  expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="卸载命令"]')!.value).toBe(runnerUninstallCommand("windows", "user"));
});
it("expires commands and permits regeneration; aborts requests on unmount", async () => {
  await act(async () => root.render(<RunnerConnection workspace="ops" inline/>));
  await click("生成安装命令");
  status.state="expired";
  await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(host.textContent).toContain("命令已过期"); expect(host.querySelector("textarea")).toBeNull();
  await click("重新生成命令");
  expect(requests.filter(r => r.init.method === "POST")).toHaveLength(2);
  const signal = requests.at(-1)!.init.signal;
  await act(async () => root.unmount()); expect(signal?.aborted).toBe(true);
  root = createRoot(host);
});

it("hides the command at local expiry even while the status service is unavailable", async () => {
  await act(async () => root.render(<RunnerConnection workspace="ops" inline/>));
  await click("生成安装命令");
  vi.stubGlobal("fetch", vi.fn(async () => { throw new Error("network failure"); }));
  await act(async () => vi.advanceTimersByTimeAsync(16000));
  expect(host.textContent).toContain("命令已过期");
  expect(host.textContent).toContain("正在重试连接状态查询");
  expect(host.querySelector("textarea")).toBeNull();
});
it("shows creation errors and lets the user retry", async () => {
  vi.stubGlobal("fetch", vi.fn(async () => ({ok:false,status:403,json:async()=>({error:"无注册权限"})})));
  await act(async () => root.render(<RunnerConnection workspace="ops" inline/>));
  await click("生成安装命令");
  expect(host.querySelector('[role="alert"]')?.textContent).toBe("无注册权限");
  expect(host.querySelector("button")?.disabled).toBe(false);
});

// Execute the exact generated block in PowerShell on native Windows CI. Mock only
// the network; execute a fixture installer in a real child PowerShell and assert
// after success, installer failure and download failure, including unset values.
it.skipIf(process.platform !== "win32")("restores Windows environment after executing the generated command", () => {
  const command = runnerInstallCommand("windows", "https://example.com", "fixture-token");
  const harness = `
$ErrorActionPreference = 'Stop'
Import-Module Microsoft.PowerShell.Management
Import-Module Microsoft.PowerShell.Utility
$global:nativePowerShell = (Get-Command powershell.exe).Source
function Invoke-WebRequest {
  param([switch]$UseBasicParsing, $Uri, $MaximumRedirection, $ErrorAction)
  if ($MaximumRedirection -ne 0) { throw 'Redirects must be disabled' }
  if ($global:scenario -eq 'download-failure') { throw 'fixture download failure' }
  return @{ Content = @'
param([string]$Server)
if ($Server -ne 'https://example.com') { throw 'Wrong server parameter' }
if ($env:FERRY_TOKEN -ne 'fixture-token') { throw 'Child token missing' }
if ($env:PSModulePath -like '*fixture-original*') { throw 'Child inherited module path' }
if ($env:RUNNER_COMMAND_TEST_CASE -eq 'installer-failure') { exit 1 }
Write-Output 'child executed'
'@ }
}
function powershell.exe {
  if ($env:PSModulePath) { throw 'Child inherited PSModulePath' }
  if ($env:FERRY_TOKEN -ne 'fixture-token') { throw 'Child token missing' }
  $global:childCalls++
  & $global:nativePowerShell @args
  $global:LASTEXITCODE = $LASTEXITCODE
}
function Start-Process {
  param($FilePath, $Verb, $ArgumentList, [switch]$PassThru)
  if ($Verb -ne 'RunAs' -or -not $PassThru) { throw 'Elevation was not requested' }
  $decoded = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($ArgumentList[-1]))
  if ($decoded.Contains('fixture-token')) { throw 'Token leaked to elevated argv' }
  $pipeName = [regex]::Match($decoded, 'ferry-runner-[0-9a-f]{32}').Value
  if (-not $pipeName) { throw 'Elevated command did not receive a pipe name' }
  $worker = @'
$pipe = [IO.Pipes.NamedPipeClientStream]::new('.', '__PIPE__', [IO.Pipes.PipeDirection]::In)
try {
  $pipe.Connect(10000)
  $reader = [IO.StreamReader]::new($pipe)
  if ($reader.ReadLine() -ne 'fixture-token') { exit 2 }
  if ($env:RUNNER_COMMAND_TEST_CASE -ne 'success') { exit 1 }
} finally { $pipe.Dispose() }
'@.Replace('__PIPE__', $pipeName)
  $global:childCalls++
  $workerEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($worker))
  return Microsoft.PowerShell.Management\\Start-Process -FilePath $global:nativePowerShell -ArgumentList @('-NoProfile', '-EncodedCommand', $workerEncoded) -PassThru -WindowStyle Hidden
}
$isAdministrator = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
foreach ($original in @('fixture-original', $null)) {
  foreach ($case in @('success', 'installer-failure', 'download-failure')) {
    $env:FERRY_TOKEN = $original
    $savedModulePath = $env:PSModulePath
    $env:PSModulePath = $original
    $global:scenario = $case
    $env:RUNNER_COMMAND_TEST_CASE = $case
    $global:childCalls = 0
    $failed = $false
    try { ${command} } catch { $failed = $true; if ($case -eq 'success') { throw } }
    if ($env:FERRY_TOKEN -ne $original) { throw 'Token was not restored' }
    if ($env:PSModulePath -ne $original) { throw 'Module path was not restored' }
    $env:PSModulePath = $savedModulePath
    if ($failed -ne ($case -ne 'success')) { throw "Unexpected result: $case" }
    $expectedCalls = $(if ($isAdministrator -and $case -eq 'download-failure') { 0 } else { 1 })
    if ($global:childCalls -ne $expectedCalls) { throw 'Child invocation count mismatch' }
  }
}
Write-Output 'environment restored'
`;
  const output = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(harness, "utf16le").toString("base64")], { encoding: "utf8", windowsHide: true, env: Object.fromEntries(Object.entries(process.env).filter(([key]) => key.toLowerCase() !== "psmodulepath")) });
  expect(output).toContain("environment restored");
});

it("requires HTTPS for remote installation but permits exact loopback HTTP origins", () => {
  for (const origin of ["http://localhost:4290", "http://127.0.0.1:4290", "http://[::1]:4290"])
    expect(runnerServerOrigin(origin)).toBe(origin);
  for (const origin of ["http://example.com", "http://192.168.1.3:4290", "http://localhost.example.com", "http://127.0.0.2", "http://0.0.0.0"])
    expect(() => runnerServerOrigin(origin)).toThrow("远程安装请使用 HTTPS，HTTP 仅支持本机测试");
  expect(() => runnerInstallCommand("linux", "http://example.com", "token")).toThrow();
  expect(() => runnerInstallCommand("windows", "http://example.com", "token")).toThrow();
});
it("regenerates consumed but expired tokens, while connected offline hosts keep waiting", async () => {
  await act(async () => root.render(<RunnerConnection workspace="ops" inline/>));
  await click("生成安装命令");
  status = {id:"t1", state:"pending", expires_at:Date.now()+15000, used_at:Date.now(), host_id:"", host_state:""};
  await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(host.textContent).toContain("已注册，等待主机上线");
  status.state = "expired";
  await act(async () => vi.advanceTimersByTimeAsync(3000));
  expect(host.textContent).toContain("命令已过期");
  const count = requests.length;
  await act(async () => vi.advanceTimersByTimeAsync(6000));
  expect(requests).toHaveLength(count);
  // The regenerated token has fresh status, not the expired predecessor's status.
  status = {id:"t1", state:"pending", expires_at:Date.now()+15000, used_at:0, host_id:"", host_state:""};
  await click("重新生成命令");
  status = {id:"t1", state:"connected", expires_at:Date.now()-1, used_at:Date.now(), host_id:"h1", host_state:"offline"};
  await act(async () => vi.advanceTimersByTimeAsync(18000));
  expect(host.textContent).toContain("已注册，等待主机上线");
  expect(host.textContent).not.toContain("命令已过期");
});
