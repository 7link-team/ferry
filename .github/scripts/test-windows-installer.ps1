$ErrorActionPreference = 'Stop'
$installer = (Get-ChildItem release-assets -Filter '*-setup.exe' | Select-Object -First 1).FullName
if (-not $installer) { throw 'NSIS installer missing' }
$config = Get-Content desktop/tauri.conf.json -Raw | ConvertFrom-Json
$registryPath = "Software\Microsoft\Windows\CurrentVersion\Uninstall\$($config.productName)"
function Run-Installer($file, $arguments) {
    Write-Output "Run installer: $file $arguments"
    $process = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
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
    Run-Installer $installer @('/S', '/UPDATE')
    $updated = Check-Scope $scope
    if ($updated.InstallLocation -ne $entry.InstallLocation) { throw 'Update changed installation directory' }
    $uninstaller = Join-Path $directory 'uninstall.exe'
    if (-not (Test-Path $uninstaller)) { throw 'Uninstaller missing' }
    Run-Installer $uninstaller @('/S', "_?=$directory")
    if (Test-Path "$($scope.Hive):\$registryPath") { throw 'Uninstall registration was not removed' }
    Write-Output "PASS: $($scope.Mode) install, update and uninstall"
}
