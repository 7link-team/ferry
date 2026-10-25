// Exercise the installed app through its real WebView2 and native Windows handles.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { chromium } from 'playwright';

const executable = process.argv[2];
assert.equal(process.platform, 'win32');
assert.ok(executable, 'installed executable required');
const pause = (ms) => new Promise(resolve => setTimeout(resolve, ms));
async function until(label, check, timeout = 45000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await pause(200);
  }
  throw new Error(`Timed out: ${label}`);
}
const port = await new Promise(resolve => {
  const listener = createServer();
  listener.listen(0, '127.0.0.1', () => {
    const value = listener.address().port;
    listener.close(() => resolve(value));
  });
});
const app = spawn(executable, [], {
  windowsHide: true,
  stdio: 'ignore',
  env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
});
app.on('error', error => { throw error; });
let browser;
function nativeWindows(closeHandle) {
  // Numeric PID/handles only; no input paths or user content in PowerShell code.
  const script = `
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class NativeWindow {
  public delegate bool Callback(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(Callback callback, IntPtr p);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder title, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint message, IntPtr w, IntPtr l);
}
'@
${closeHandle ? `[NativeWindow]::PostMessage([IntPtr]${BigInt(closeHandle)}, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null` : ''}
$result = [System.Collections.Generic.List[object]]::new()
[NativeWindow]::EnumWindows({ param($handle, $unused)
  $owner = [uint32]0
  [NativeWindow]::GetWindowThreadProcessId($handle, [ref]$owner) | Out-Null
  if ($owner -eq ${app.pid}) {
    $title = [System.Text.StringBuilder]::new(1024)
    [NativeWindow]::GetWindowText($handle, $title, 1024) | Out-Null
    if ($title.Length -gt 0) { $result.Add(@{ handle=$handle.ToInt64().ToString(); title=$title.ToString(); visible=[NativeWindow]::IsWindowVisible($handle) }) }
  }
  return $true
}, [IntPtr]::Zero) | Out-Null
ConvertTo-Json -InputObject @($result.ToArray()) -Compress
`;
  const output = execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script],
    { windowsHide: true, encoding: 'utf8', timeout: 15000 });
  return JSON.parse(output.trim());
}
try {
  await until('WebView2 debugging endpoint', async () => {
    assert.equal(app.exitCode, null, 'app exited before opening launcher');
    try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; } catch { return false; }
  });
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  const pages = () => browser.contexts().flatMap(context => context.pages());
  const launcher = await until('launcher webview', async () => {
    for (const page of pages()) if (await page.locator('#open').count()) return page;
  });
  const launcherHandle = nativeWindows().find(window => window.visible)?.handle;
  assert.ok(launcherHandle, 'launcher is a visible native window');
  await launcher.locator('#url').fill('');
  await launcher.locator('#open').click();
  const workspace = await until('local workspace webview', async () => {
    const error = await launcher.locator('#connection-error').textContent();
    assert.equal(error, '', `launcher startup error: ${error}`);
    return pages().find(page => page.url().startsWith('http://127.0.0.1:4290'));
  });
  await workspace.locator('input[type="password"]').waitFor();
  const status = await (await fetch('http://127.0.0.1:4290/api/status')).json();
  assert.equal(status.ok, true);
  assert.equal(status.server_mode, false);
  const windows = await until('launcher hidden and workspace visible', () => {
    const all = nativeWindows();
    return all.find(w => w.handle === launcherHandle && !w.visible)
      && all.find(w => w.handle !== launcherHandle && w.visible) && all;
  });
  const workspaceHandle = windows.find(w => w.handle !== launcherHandle && w.visible).handle;
  nativeWindows(workspaceHandle);
  await until('launcher restored after workspace close', () => nativeWindows().find(w => w.handle === launcherHandle && w.visible));
  // Reopening must reuse the service and preserve its identity.
  await launcher.locator('#open').click();
  await until('workspace reopened', () => nativeWindows().find(w => w.handle !== launcherHandle && w.visible));
  assert.equal((await (await fetch('http://127.0.0.1:4290/api/status')).json()).instance_id, status.instance_id);
  const reopened = nativeWindows().find(w => w.handle !== launcherHandle && w.visible);
  nativeWindows(reopened.handle);
  await until('launcher restored again', () => nativeWindows().find(w => w.handle === launcherHandle && w.visible));
  nativeWindows(launcherHandle);
  await until('normal app exit', () => app.exitCode !== null);
  await until('owned server stops on app exit', async () => {
    try { await fetch('http://127.0.0.1:4290/api/status', { signal: AbortSignal.timeout(1000) }); return false; }
    catch { return true; }
  });
  console.log('PASS: installed blank-address launch, real login page, launcher hide/restore, reopen, normal exit and service cleanup');
} catch (error) {
  console.error('Desktop smoke failure:', error);
  console.error('Native windows:', nativeWindows());
  console.error('Webviews:', browser?.contexts().flatMap(context => context.pages().map(page => page.url())));
  throw error;
} finally {
  await browser?.close().catch(() => {});
  if (app.exitCode === null) {
    execFileSync('taskkill.exe', ['/PID', String(app.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
  }
}
