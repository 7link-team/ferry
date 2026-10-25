$ErrorActionPreference = 'Stop'
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
function Check-Scope($scope) {
    $entry = Get-ItemProperty "$($scope.Hive):\$registryPath" -ErrorAction SilentlyContinue
    if (-not $entry) { throw "$($scope.Hive) registration missing" }
    if (Test-Path "$($scope.OtherHive):\$registryPath") { throw 'Installation registered under both scopes' }
    # NSIS quotes InstallLocation. MultiUser initializes its own default directory,
    # overriding /D during .onInit; exercise the actual user/machine destinations.
    $directory = $entry.InstallLocation.Trim('"').TrimEnd('\')
    if (-not $directory.StartsWith($scope.Root.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Wrong installation scope directory: $directory (expected under $($scope.Root))"
    }
    if ($entry.DisplayVersion -ne $config.version) { throw "Wrong installed version: $($entry.DisplayVersion)" }
    if (-not (Test-Path (Join-Path $directory $entry.MainBinaryName))) { throw 'Application executable missing' }
    Write-Host "Verified $($scope.Hive) registration and executable at $directory"
    return $entry
}
foreach ($scope in @(@{ Mode = 'CurrentUser'; Hive = 'HKCU'; OtherHive = 'HKLM'; Root = $env:LOCALAPPDATA }, @{ Mode = 'AllUsers'; Hive = 'HKLM'; OtherHive = 'HKCU'; Root = $env:ProgramFiles })) {
    Run-Installer $installer @('/S', "/$($scope.Mode)")
    $entry = Check-Scope $scope
    $directory = $entry.InstallLocation.Trim('"').TrimEnd('\')
    & node .github/scripts/test-desktop-windows.mjs (Join-Path $directory $entry.MainBinaryName)
    if ($LASTEXITCODE -ne 0) { throw 'Installed desktop launch/lifecycle verification failed' }
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
        if (-not $restarted.CloseMainWindow()) { throw 'Could not close restarted app' }
        if (-not $restarted.WaitForExit(30000)) { throw 'Restarted app did not exit normally' }
    }
    $updated = Check-Scope $scope
    if ($updated.InstallLocation -ne $entry.InstallLocation) { throw 'Update changed installation directory' }
    $uninstaller = Join-Path $directory 'uninstall.exe'
    if (-not (Test-Path $uninstaller)) { throw 'Uninstaller missing' }
    Run-Installer $uninstaller @('/S', "_?=$directory")
    if (Test-Path "$($scope.Hive):\$registryPath") { throw 'Uninstall registration was not removed' }
    Write-Output "PASS: $($scope.Mode) install, update and uninstall"
}
