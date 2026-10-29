$ErrorActionPreference = 'Stop'
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
                if ($restarted -and $restarted.MainWindowHandle -ne 0) { break }
                Start-Sleep -Milliseconds 200
            }
            if (-not $restarted -or $restarted.MainWindowHandle -eq 0) { throw 'Update did not automatically restart application' }
            # Closing the remembered workspace restores settings; close that window as well.
            $closeDeadline = (Get-Date).AddSeconds(30)
            while (-not $restarted.HasExited -and (Get-Date) -lt $closeDeadline) {
                $restarted.Refresh()
                if ($restarted.MainWindowHandle -ne 0) { [void]$restarted.CloseMainWindow() }
                if ($restarted.WaitForExit(500)) { break }
            }
            if (-not $restarted.HasExited) { throw 'Restarted app did not exit normally' }
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
