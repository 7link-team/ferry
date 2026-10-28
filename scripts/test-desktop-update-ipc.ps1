param([Parameter(Mandatory=$true)][string]$TestExecutable)
$ErrorActionPreference = 'Stop'
$tag = [guid]::NewGuid().ToString('N').Substring(0,12)
$user = "upd$tag"
$serverTask = "DesktopUpdateServer-$tag"
$clientTask = "DesktopUpdateClient-$tag"
$root = Join-Path $env:ProgramData "desktop-update-test-$tag"
$password = "Test!$([guid]::NewGuid().ToString('N'))a1"
try {
    New-Item -ItemType Directory -Path $root | Out-Null
    & icacls.exe $root /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX' | Out-Null
    if ($LASTEXITCODE) { throw 'Protect fixture directory failed' }
    $result = New-Item -ItemType Directory -Path (Join-Path $root 'client-result')
    & icacls.exe $result.FullName /grant '*S-1-5-32-545:(OI)(CI)M' | Out-Null
    if ($LASTEXITCODE) { throw 'Create client result directory failed' }
    Copy-Item -LiteralPath $TestExecutable -Destination (Join-Path $root 'test.exe')
    New-LocalUser -Name $user -Password (ConvertTo-SecureString $password -AsPlainText -Force) -AccountNeverExpires -PasswordNeverExpires | Out-Null
    Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $user
    foreach ($kind in @('server','client')) {
        $test = if ($kind -eq 'server') { 'system_server_fixture' } else { 'standard_client_fixture' }
        $output = if ($kind -eq 'server') { $root } else { $result.FullName }
        @"
@echo off
set "DESKTOP_IPC_TEST_ROOT=$root"
"$root\test.exe" --exact ipc::tests::$test --nocapture > "$output\test.log" 2>&1
echo %ERRORLEVEL% > "$output\exit.txt"
"@ | Set-Content -LiteralPath (Join-Path $root "$kind.cmd") -Encoding ascii
    }
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 2)
    $action = New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\cmd.exe" -Argument "/d /c `"$root\server.cmd`""
    Register-ScheduledTask -TaskName $serverTask -Action $action -Settings $settings -User 'SYSTEM' -RunLevel Highest | Out-Null
    Start-ScheduledTask -TaskName $serverTask
    $deadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path (Join-Path $root 'ready.json')) -and (Get-Date) -lt $deadline) {
        if (Test-Path (Join-Path $root 'exit.txt')) { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path (Join-Path $root 'ready.json'))) { throw 'SYSTEM pipe server failed to become ready' }
    $action = New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\cmd.exe" -Argument "/d /c `"$root\client.cmd`""
    Register-ScheduledTask -TaskName $clientTask -Action $action -Settings $settings -User "$env:COMPUTERNAME\$user" -Password $password -RunLevel Limited | Out-Null
    Start-ScheduledTask -TaskName $clientTask
    $deadline = (Get-Date).AddSeconds(55)
    while ((-not (Test-Path (Join-Path $result.FullName 'exit.txt')) -or -not (Test-Path (Join-Path $root 'exit.txt'))) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 100 }
    foreach ($output in @($root, $result.FullName)) {
        $exit = Join-Path $output 'exit.txt'
        if (-not (Test-Path $exit)) { throw "Fixture did not exit: $output" }
        if ((Get-Content $exit -Raw).Trim() -ne '0') { throw "Native IPC fixture failed: $output" }
    }
    Write-Host 'PASS: SYSTEM server / standard-user pipe transfer, peer identities and spoof rejection'
} finally {
    foreach ($log in @((Join-Path $root 'test.log'), (Join-Path $root 'client-result/test.log'))) {
        if (Test-Path $log) { Get-Content $log }
    }
    foreach ($task in @($serverTask, $clientTask)) {
        if (Get-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue) {
            Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
            Unregister-ScheduledTask -TaskName $task -Confirm:$false
        }
    }
    if (Get-LocalUser -Name $user -ErrorAction SilentlyContinue) { Remove-LocalUser -Name $user }
    if (Test-Path $root) { Remove-Item -Recurse -Force $root }
}
