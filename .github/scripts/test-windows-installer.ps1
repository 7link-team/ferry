$ErrorActionPreference = 'Stop'
$installer = (Get-ChildItem release-assets -Filter '*-setup.exe' | Select-Object -First 1).FullName
if (-not $installer) { throw 'NSIS installer missing' }
$root = Join-Path $env:RUNNER_TEMP 'installer-scope-check'
New-Item -ItemType Directory -Force $root | Out-Null
function Run-Installer($file, $arguments) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) { throw "Installer failed with exit code $($process.ExitCode)" }
}
function Check-Scope($hive, $directory) {
    $entries = @(Get-ItemProperty "$hive`:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue | Where-Object { $_.InstallLocation -and $_.InstallLocation.Trim('"').TrimEnd('\') -eq $directory.TrimEnd('\') })
    if ($entries.Count -ne 1) { throw "$hive registration missing or duplicated for $directory" }
    if (-not (Get-ChildItem $directory -Filter '*.exe' | Where-Object { $_.Name -notlike '*uninstall*' })) { throw 'Application executable missing' }
    return $entries[0]
}
foreach ($scope in @(@{ Mode = 'CurrentUser'; Hive = 'HKCU'; Dir = 'user' }, @{ Mode = 'AllUsers'; Hive = 'HKLM'; Dir = 'machine' })) {
    $directory = Join-Path $root $scope.Dir
    Run-Installer $installer @('/S', "/$($scope.Mode)", "/D=$directory")
    $entry = Check-Scope $scope.Hive $directory
    # Exercise the updater's reinstall path and ensure the chosen installation scope remains.
    Run-Installer $installer @('/S', '/UPDATE', "/D=$directory")
    $entry = Check-Scope $scope.Hive $directory
    $uninstaller = (Get-ChildItem $directory -Filter '*uninstall*.exe' | Select-Object -First 1).FullName
    if (-not $uninstaller) { throw 'Uninstaller missing' }
    Run-Installer $uninstaller @('/S', "_?=$directory")
    $remaining = @(Get-ItemProperty "$($scope.Hive):\Software\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue | Where-Object { $_.InstallLocation -and $_.InstallLocation.Trim('"').TrimEnd('\') -eq $directory.TrimEnd('\') })
    if ($remaining.Count -ne 0) { throw 'Uninstall registration was not removed' }
    Write-Output "PASS: $($scope.Mode) install, update and uninstall"
}
