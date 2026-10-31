$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class InstallerWindowDiagnostics {
    public delegate bool Callback(IntPtr hwnd, IntPtr state);
    [DllImport("user32.dll")] public static extern bool EnumWindows(Callback callback, IntPtr state);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr hwnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int length);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int length);
    [DllImport("user32.dll", SetLastError=true)] private static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    public static string CloseProbe(IntPtr hwnd) {
        bool accepted = PostMessage(hwnd, 0x0010, IntPtr.Zero, IntPtr.Zero);
        return accepted + ":" + Marshal.GetLastWin32Error();
    }
    public static IntPtr[] ApplicationWindows(int processId) {
        var windows = new System.Collections.Generic.List<IntPtr>();
        EnumWindows((hwnd, state) => {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            if (owner == (uint)processId && IsWindowVisible(hwnd) && IsWindowEnabled(hwnd)) {
                var name = new StringBuilder(256);
                GetClassName(hwnd, name, name.Capacity);
                if (name.ToString() == "Tauri Window") windows.Add(hwnd);
            }
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }
}
'@
try {
$installer = (Get-ChildItem release-assets -Filter '*-setup.exe' | Select-Object -First 1).FullName
if (-not $installer) { throw 'NSIS installer missing' }
$config = Get-Content desktop/tauri.conf.json -Raw | ConvertFrom-Json
$registryPath = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$($config.productName)"
function Run-Installer($file, $arguments) {
    Write-Output "Run installer: $file $arguments"
    $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru -WindowStyle Hidden
    if (-not $process.WaitForExit(120000)) {
        Stop-Process -Id $process.Id -Force
        throw 'Installer did not finish unattended within 120 seconds'
    }
    if ($process.ExitCode -ne 0) { throw "Installer failed with exit code $($process.ExitCode)" }
}
function Check-Scope($scope, $expectedVersion = $config.version) {
    $entry = Get-ItemProperty "$($scope.Hive):\$registryPath" -ErrorAction SilentlyContinue
    if (-not $entry) { throw "$($scope.Hive) registration missing" }
    if (Test-Path "$($scope.OtherHive):\$registryPath") { throw 'Installation registered under both scopes' }
    # NSIS quotes InstallLocation. MultiUser initializes its own default directory,
    # overriding /D during .onInit; exercise the actual user/machine destinations.
    $directory = $entry.InstallLocation.Trim('"').TrimEnd('\')
    if (-not $directory.StartsWith($scope.Root.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Wrong installation scope directory: $directory (expected under $($scope.Root))"
    }
    if ($entry.DisplayVersion -ne $expectedVersion) { throw "Wrong installed version: $($entry.DisplayVersion)" }
    if (-not (Test-Path (Join-Path $directory $entry.MainBinaryName))) { throw 'Application executable missing' }
    Write-Host "Verified $($scope.Hive) registration and executable at $directory"
    return $entry
}
# Exercise both the original /UPDATE-only client and the latest published alpha.
$previousInstallers = @{}
foreach ($previousVersion in @('0.1.0-alpha.4', '0.1.0-alpha.6', '0.1.0-alpha.7')) {
    $previousRelease = Invoke-RestMethod "https://api.github.com/repos/$($config.plugins.release.repository)/releases/tags/v$previousVersion"
    $previousAsset = $previousRelease.assets | Where-Object { $_.name.EndsWith('_x64-setup.exe') } | Select-Object -First 1
    if (-not $previousAsset) { throw "Previous Windows installer missing: $previousVersion" }
    $previousInstaller = Join-Path $env:RUNNER_TEMP "previous-$previousVersion-setup.exe"
    Invoke-WebRequest $previousAsset.browser_download_url -OutFile $previousInstaller
    $previousInstallers[$previousVersion] = $previousInstaller
}
foreach ($scope in @(@{ Mode = 'CurrentUser'; Hive = 'HKCU'; OtherHive = 'HKLM'; Root = $env:LOCALAPPDATA }, @{ Mode = 'AllUsers'; Hive = 'HKLM'; OtherHive = 'HKCU'; Root = $env:ProgramFiles })) {
    Run-Installer $installer @('/S', "/$($scope.Mode)")
    $entry = Check-Scope $scope
    $directory = $entry.InstallLocation.Trim('"').TrimEnd('\')
    & node .github/scripts/test-desktop-windows.mjs (Join-Path $directory $entry.MainBinaryName)
    if ($LASTEXITCODE -ne 0) { throw 'Installed desktop launch/lifecycle verification failed' }
    Run-Installer (Join-Path $directory 'uninstall.exe') @('/S', "_?=$directory")
    foreach ($previousVersion in @('0.1.0-alpha.4', '0.1.0-alpha.6', '0.1.0-alpha.7')) {
        $previousInstaller = $previousInstallers[$previousVersion]
        Run-Installer $previousInstaller @('/S', "/$($scope.Mode)")
        $entry = Check-Scope $scope $previousVersion
        $directory = $entry.InstallLocation.Trim('"').TrimEnd('\')
        # Old basicUi clients pass only /UPDATE. New passive clients add /P /R.
        # Both must finish without a maintenance choice and automatically launch the app.
        foreach ($updateArgs in @(@('/UPDATE'), @('/P', '/R', '/UPDATE'))) {
            Run-Installer $installer $updateArgs
            $deadline = (Get-Date).AddSeconds(45)
            $restarted = $null
            while ((Get-Date) -lt $deadline) {
                $restarted = Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($entry.MainBinaryName)) -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq (Join-Path $directory $entry.MainBinaryName) } | Select-Object -First 1
                if ($restarted -and [InstallerWindowDiagnostics]::ApplicationWindows($restarted.Id).Length -gt 0) { break }
                Start-Sleep -Milliseconds 200
            }
            if (-not $restarted -or [InstallerWindowDiagnostics]::ApplicationWindows($restarted.Id).Length -eq 0) { throw 'Update did not automatically restart application' }
            # Closing the remembered workspace restores settings; close that window as well.
            $closeDeadline = (Get-Date).AddSeconds(30)
            while (-not $restarted.HasExited -and (Get-Date) -lt $closeDeadline) {
                $restarted.Refresh()
                # MainWindowHandle can select the single-instance plugin's invisible
                # event target (-siw), whose WS_VISIBLE style is only for WM_PAINT.
                # Close an actual visible Tauri UI window, as the native UI gate does.
                $windows = [InstallerWindowDiagnostics]::ApplicationWindows($restarted.Id)
                if ($windows.Length -gt 0) {
                    $windowHandle = $windows[0]
                    $closed = [InstallerWindowDiagnostics]::CloseProbe($windowHandle)
                    Write-Output "Close restarted application window: PID=$($restarted.Id) HWND=$windowHandle accepted:Win32Error=$closed"
                }
                if ($restarted.WaitForExit(500)) { break }
            }
            if (-not $restarted.HasExited) {

                Write-Host "Final close probe accepted:Win32Error=$([InstallerWindowDiagnostics]::CloseProbe($restarted.MainWindowHandle))"
                [InstallerWindowDiagnostics]::EnumWindows({ param($handle, $state)
                    $windowProcess = [uint32]0
                    [void][InstallerWindowDiagnostics]::GetWindowThreadProcessId($handle, [ref]$windowProcess)
                    if ($windowProcess -eq $restarted.Id) {
                        $title = [Text.StringBuilder]::new(512)
                        $class = [Text.StringBuilder]::new(256)
                        [void][InstallerWindowDiagnostics]::GetWindowText($handle, $title, $title.Capacity)
                        [void][InstallerWindowDiagnostics]::GetClassName($handle, $class, $class.Capacity)
                        Write-Host "Remaining window: HWND=$handle class=$class title=$title visible=$([InstallerWindowDiagnostics]::IsWindowVisible($handle)) enabled=$([InstallerWindowDiagnostics]::IsWindowEnabled($handle))"
                    }
                    return $true
                }, [IntPtr]::Zero) | Out-Null
                throw 'Restarted app did not exit normally'
            }
        }
        $updated = Check-Scope $scope
        if ($updated.InstallLocation -ne $entry.InstallLocation) { throw 'Update changed installation directory' }
        $uninstaller = Join-Path $directory 'uninstall.exe'
        if (-not (Test-Path $uninstaller)) { throw 'Uninstaller missing' }
        Run-Installer $uninstaller @('/S', "_?=$directory")
        if (Test-Path "$($scope.Hive):\$registryPath") { throw 'Uninstall registration was not removed' }
        Write-Output "PASS: $($scope.Mode) install, update and uninstall"
    }
}
} catch {
    $detail = $_.Exception.Message.Replace('%', '%25').Replace("`r", '%0D').Replace("`n", '%0A')
    Write-Output "::error::$detail"
    throw
}
