// Exercise the installed app through its real WebView2 and native Windows handles.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { chromium } from 'playwright';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

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
const startApp = () => spawn(executable, [], {
  windowsHide: true,
  stdio: 'ignore',
  env: { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
});
let app = startApp();
app.on('error', error => { throw error; });
let browser;
function nativeWindows(closeHandle) {
  // Numeric PID/handles only; no input paths or user content in PowerShell code.
  const script = `
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
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
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string title);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string c, string title);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint message, IntPtr w, IntPtr l);
  public static void OpenNotificationOverflow() {
    // Icons in the collapsed Windows overflow have no visible rectangle.
    // Open the real shell's chevron before asking the shell for their geometry.
    var overflow = FindWindow("NotifyIconOverflowWindow", null);
    if (overflow != IntPtr.Zero && IsWindowVisible(overflow)) return;
    var taskbar = FindWindow("Shell_TrayWnd", null);
    var notifications = FindWindowEx(taskbar, IntPtr.Zero, "TrayNotifyWnd", null);
    var chevron = FindWindowEx(notifications, IntPtr.Zero, "Button", null);
    if (notifications != IntPtr.Zero && chevron != IntPtr.Zero)
      SendMessage(chevron, 0x00F5, IntPtr.Zero, IntPtr.Zero);
  }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder name, int n);
  [StructLayout(LayoutKind.Sequential)] public struct IconId { public uint size; public IntPtr hwnd; public uint id; public Guid guid; }
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int left, top, right, bottom; }
  [DllImport("shell32.dll")] public static extern int Shell_NotifyIconGetRect(ref IconId id, out Rect rect);
  public static bool HasTrayIcon(IntPtr h) {
    // The builder consumes one counter before allocating the native icon ID.
    // Discover an actual registered icon on this owned tray window, not a guessed ID.
    for (uint number = 1; number <= 32; number++) {
      var id = new IconId { size=(uint)Marshal.SizeOf(typeof(IconId)), hwnd=h, id=number };
      Rect rect;
      if (Shell_NotifyIconGetRect(ref id, out rect) == 0) return true;
    }
    return false;
  }
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint message, IntPtr w, IntPtr l);
}
'@
[NativeWindow]::OpenNotificationOverflow()
${closeHandle ? `[NativeWindow]::PostMessage([IntPtr]${BigInt(closeHandle)}, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null` : ''}
$result = [System.Collections.Generic.List[object]]::new()
[NativeWindow]::EnumWindows({ param($handle, $unused)
  $owner = [uint32]0
  [NativeWindow]::GetWindowThreadProcessId($handle, [ref]$owner) | Out-Null
  if ($owner -eq ${app.pid}) {
    $title = [System.Text.StringBuilder]::new(1024)
    [NativeWindow]::GetWindowText($handle, $title, 1024) | Out-Null
    $class = [System.Text.StringBuilder]::new(256)
    [NativeWindow]::GetClassName($handle, $class, 256) | Out-Null
    if ($class.ToString() -eq 'Tauri Window' -or $class.ToString() -eq 'tray_icon_app') { $result.Add(@{ handle=$handle.ToInt64().ToString(); title=$title.ToString(); visible=[NativeWindow]::IsWindowVisible($handle); tray=($class.ToString() -eq 'tray_icon_app' -and [NativeWindow]::HasTrayIcon($handle)) }) }
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
  await until('registered Windows notification icon', () => nativeWindows().some(window => window.tray));
  await launcher.locator('#url').fill('');
  await launcher.locator('#open').click();
  const workspace = await until('local workspace webview', async () => {
    const error = await launcher.locator('#connection-error').textContent();
    assert.equal(error, '', `launcher startup error: ${error}`);
    return pages().find(page => page.url().startsWith('http://127.0.0.1:4290'));
  });
  await workspace.getByRole('heading', { name: '初始化平台超级管理员' }).waitFor();
  assert.equal(await workspace.locator('input[name="token"]').count(), 0, 'owned sidecar supplies operator proof automatically');
  assert.equal(await workspace.locator('input[type="password"]').count(), 2, 'setup requires password confirmation');
  assert.equal(new URL(workspace.url()).hash, '', 'setup token removed from browser URL');
  const status = await (await fetch('http://127.0.0.1:4290/api/status')).json();
  assert.equal(status.ok, true);
  assert.equal(status.server_mode, false);
  const windows = await until('launcher hidden and workspace visible', () => {
    const all = nativeWindows();
    return all.find(w => w.handle === launcherHandle && !w.visible)
      && all.find(w => w.handle !== launcherHandle && w.visible) && all;
  });
  const workspaceHandle = windows.find(w => w.handle !== launcherHandle && w.visible).handle;
  // A second launch must focus the workspace without resurfacing settings.
  const second = spawn(executable, [], { windowsHide: true, stdio: 'ignore' });
  await until('second instance exits', () => second.exitCode !== null);
  assert.equal(nativeWindows().find(w => w.handle === launcherHandle).visible, false);
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
  await browser.close().catch(() => {});
  const config = JSON.parse(readFileSync('desktop/tauri.conf.json', 'utf8'));
  const data = join(process.env.LOCALAPPDATA, config.identifier);
  mkdirSync(data, { recursive: true });
  for (const restartKind of ['saved connection', 'upgrade resume']) {
    if (restartKind === 'upgrade resume') writeFileSync(join(data, 'update-resume.json'), JSON.stringify({ server_url: '' }));
    app = startApp();
    app.on('error', error => { throw error; });
    await until('restarted WebView2 endpoint', async () => {
      try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; } catch { return false; }
    });
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    const resumed = await until('automatic workspace resume', () => pages().find(page => page.url().startsWith('http://127.0.0.1:4290')));
    await resumed.getByRole('heading', { name: '初始化平台超级管理员' }).waitFor();
    assert.equal(await resumed.locator('input[name="token"]').count(), 0, 'resumed sidecar supplies setup proof');
    assert.equal((await (await fetch('http://127.0.0.1:4290/api/status')).json()).instance_id, status.instance_id);
    const resumedWindows = await until('only resumed workspace visible', () => {
      const all = nativeWindows();
      return all.filter(w => w.visible).length === 1 && all.some(w => w.title.includes('连接设置') && !w.visible) && all;
    });
    nativeWindows(resumedWindows.find(w => w.visible).handle);
    const settings = await until('resumed settings restored', () => nativeWindows().find(w => w.title.includes('连接设置') && w.visible));
    nativeWindows(settings.handle);
    await until('resumed app exit', () => app.exitCode !== null);
    await until('resumed server exit', async () => {
      try { await fetch('http://127.0.0.1:4290/api/status', { signal: AbortSignal.timeout(1000) }); return false; } catch { return true; }
    });
    await browser.close().catch(() => {});
    console.log(`PASS: automatic ${restartKind} without reopening connection settings`);
  }
  console.log('PASS: tray registration, blank-address login, launcher hide/restore, single instance, automatic upgrade resume, persistent identity and service cleanup');
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
