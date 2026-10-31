param(
    [Parameter(Mandatory = $true)][string]$Server,
    [string]$Name = 'runner-agent',
    # Install for the current user as a logon task: no elevation, the user's own
    # identity, profile and PATH; runs only while that user is logged on.
    [switch]$User
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Write-Host "Preparing Runner installation from $Server..."
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $User -and -not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open PowerShell as Administrator and run the installation command again.'
}
$serverUri = [Uri]$Server
if (-not $serverUri.IsAbsoluteUri -or $serverUri.Scheme -notin @('http', 'https') -or $serverUri.UserInfo -or $serverUri.Query -or $serverUri.Fragment) { throw 'Server must be an HTTP(S) URL without credentials, query or fragment.' }
# Server-verified metadata must itself arrive over an authenticated connection.
if ($serverUri.Scheme -eq 'http' -and $Server -cnotmatch '^http://(?:localhost|127\.0\.0\.1|\[::1\])(?::[0-9]{1,5})?(?:/|$)') {
    throw 'Remote servers require HTTPS; HTTP is allowed only on localhost, 127.0.0.1 or [::1].'
}
if ($Name -cnotmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$') { throw 'Invalid service name.' }
if (-not $env:FERRY_TOKEN -or $env:FERRY_TOKEN -cnotmatch '^[a-zA-Z0-9._~-]+$') { throw 'Set FERRY_TOKEN to the enrolment token first.' }
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64') { throw 'Only Windows x64 is supported.' }

# Never stage a privileged executable in a user-writable temporary directory.
# A current-user installation stages in that user's own profile instead.
$admins = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
$system = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$trustedInstaller = ([Security.Principal.NTAccount]::new('NT SERVICE\TrustedInstaller')).Translate([Security.Principal.SecurityIdentifier]).Value
$trusted = @($admins.Value, $system.Value, $identity.User.Value, $trustedInstaller)
$replace = [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership
$write = $replace -bor [Security.AccessControl.FileSystemRights]::Write
$base = Get-Item -LiteralPath ([Environment]::GetFolderPath($(if ($User) { 'LocalApplicationData' } else { 'ProgramFiles' }))) -Force
$item = $base
while ($item) {
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Reparse points are not allowed in the installation staging path.' }
    $acl = Get-Acl -LiteralPath $item.FullName
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trusted) { throw 'Untrusted installation staging path owner.' }
    $mask = if ($item.FullName -eq $base.FullName) { $write } else { $replace }
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) { continue }
        if ($rule.IdentityReference.Value -eq 'S-1-3-4') { continue }
        if ($rule.AccessControlType -eq [Security.AccessControl.AccessControlType]::Allow -and $rule.IdentityReference.Value -notin $trusted -and ($rule.FileSystemRights -band $mask)) { throw 'Untrusted write access to installation staging path.' }
    }
    $item = $item.Parent
}
$stage = Join-Path $base.FullName ('runner-install-' + [Guid]::NewGuid().ToString('N'))
$client = $null
try {
    $null = New-Item -ItemType Directory -Path $stage
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner($(if ($User) { $identity.User } else { $admins }))
    foreach ($sid in $(if ($User) { @($identity.User, $system) } else { @($admins, $system) })) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
    }
    # Set-Acl also writes the audit SACL, which an unprivileged user cannot.
    if ($User) { (Get-Item -LiteralPath $stage -Force).SetAccessControl($acl) } else { Set-Acl -LiteralPath $stage -AclObject $acl }
    Add-Type -AssemblyName System.Net.Http
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(300)
    function Save-Download([Uri]$Url, [string]$Path, [long]$Limit, [bool]$Metadata) {
        $originalUrl = $Url
        $attempt = 0
        while ($true) {
          $attempt++
          $Url = $originalUrl
          [long]$offset = if (-not $Metadata -and (Test-Path -LiteralPath $Path)) { (Get-Item -LiteralPath $Path).Length } else { 0 }
          if (-not $Metadata -and $offset -eq $Limit) { return }
          if ($offset -gt $Limit) { throw 'Download exceeds size limit.' }
          $retryable = $false
          $deadline = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds($(if ($Metadata) { 60 } else { 300 })))
          try {
            for ($redirect = 0; $redirect -le 5; $redirect++) {
                if (-not $Metadata -and $Url.Scheme -ne 'https') { throw 'Artifact URL and redirects must use HTTPS.' }
                $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::Get, $Url)
                $response = $null
                try {
                    if ($Metadata) { $request.Headers.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $env:FERRY_TOKEN) }
                    if ($offset -gt 0) { $request.Headers.Range = [Net.Http.Headers.RangeHeaderValue]::new($offset, $null) }
                    $retryable = $true
                    $response = $client.SendAsync($request, [Net.Http.HttpCompletionOption]::ResponseHeadersRead, $deadline.Token).GetAwaiter().GetResult()
                    $retryable = $false
                    if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                        if ($Metadata -or $redirect -eq 5 -or -not $response.Headers.Location) { throw 'Unexpected download redirect.' }
                        $Url = [Uri]::new($Url, $response.Headers.Location)
                        continue
                    }
                    $status = [int]$response.StatusCode
                    if (-not $response.IsSuccessStatusCode) {
                        # Expired redirected artifact links are refreshed from the original release URL.
                        $retryable = $status -in @(408, 429) -or $status -ge 500 -or (-not $Metadata -and $status -in @(401, 403))
                        throw "Download returned HTTP $status."
                    }
                    if ($status -eq 206) {
                        $range = $response.Content.Headers.ContentRange
                        if ($Metadata -or -not $range -or $range.Unit -ne 'bytes' -or $range.From -ne $offset -or $range.Length -ne $Limit -or $range.To -ge $Limit) { throw 'Invalid download Content-Range; nothing installed.' }
                    } elseif ($status -eq 200) {
                        if ($offset -gt 0) { Write-Host 'Server does not support resume; restarting download.' }
                        $offset = 0
                    } else { throw "Unexpected download HTTP $status." }
                    if ($response.Content.Headers.ContentLength -gt ($Limit - $offset)) { throw 'Download exceeds size limit.' }
                    $retryable = $true
                    $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                    $retryable = $false
                    $mode = if ($offset -gt 0) { [IO.FileMode]::Append } else { [IO.FileMode]::Create }
                    $outputStream = [IO.File]::Open($Path, $mode, [IO.FileAccess]::Write, [IO.FileShare]::None)
                    try {
                        $buffer = [byte[]]::new(65536)
                        [long]$total = $offset
                        $progress = [Diagnostics.Stopwatch]::StartNew()
                        while ($true) {
                            $retryable = $true
                            $count = $inputStream.ReadAsync($buffer, 0, $buffer.Length, $deadline.Token).GetAwaiter().GetResult()
                            $retryable = $false
                            if ($count -eq 0) { break }
                            $total += $count
                            if ($total -gt $Limit) { throw 'Download exceeds size limit.' }
                            $outputStream.Write($buffer, 0, $count)
                            if (-not $Metadata -and $progress.Elapsed.TotalSeconds -ge 3) {
                                Write-Host ('Downloaded {0:N1} / {1:N1} MiB...' -f ($total / 1MB), ($Limit / 1MB))
                                $progress.Restart()
                            }
                        }
                        if ((-not $Metadata -and $total -ne $Limit) -or ($null -ne $response.Content.Headers.ContentLength -and ($total - $offset) -ne $response.Content.Headers.ContentLength)) {
                            $retryable = $true
                            throw 'Download interrupted before all bytes arrived.'
                        }
                    } finally { $outputStream.Dispose(); $inputStream.Dispose() }
                    return
                } finally { if ($response) { $response.Dispose() }; $request.Dispose() }
            }
          } catch {
            if ($_.Exception -is [Management.Automation.PipelineStoppedException]) { throw }
            if (-not $retryable) { throw }
            $delay = [Math]::Min(10, $attempt * 2)
            [long]$saved = if (-not $Metadata -and (Test-Path -LiteralPath $Path)) { (Get-Item -LiteralPath $Path).Length } else { 0 }
            Write-Host ('Download interrupted. Retry {0} in {1}s; saved {2:N1} MiB. Press Ctrl+C to cancel.' -f $attempt, $delay, ($saved / 1MB))
            Start-Sleep -Seconds $delay
          } finally { $deadline.Dispose() }
        }
    }
    $metadataPath = Join-Path $stage 'metadata'
    Write-Host 'Fetching verified release metadata...'
    Save-Download ($Server.TrimEnd('/') + '/api/runners/install/x86_64-pc-windows-msvc') $metadataPath 8192 $true
    $metadata = [IO.File]::ReadAllText($metadataPath)
    $lines = $metadata.Split([char]10)
    if ($lines.Length -ne 5 -or $lines[4] -ne '' -or $lines[0] -cnotmatch '^[a-zA-Z0-9.+-]+$' -or $lines[2] -cnotmatch '^[0-9a-f]{64}$' -or $lines[3] -cnotmatch '^[1-9][0-9]{0,8}$') { throw 'Invalid installation metadata.' }
    $url = [Uri]$lines[1]
    if (-not $url.IsAbsoluteUri -or $url.Scheme -ne 'https') { throw 'Artifact URL must use HTTPS.' }
    $size = [long]$lines[3]
    if ($size -gt 268435456) { throw 'Artifact exceeds size limit.' }
    $binary = Join-Path $stage 'runner.exe'
    Write-Host ('Downloading Runner {0} ({1:N1} MiB)...' -f $lines[0], ($size / 1MB))
    Save-Download $url $binary $size $false
    Write-Host 'Verifying downloaded Runner size and SHA256...'
    if ((Get-Item -LiteralPath $binary).Length -ne $size) { throw 'Artifact size mismatch; nothing installed.' }
    if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -cne $lines[2]) { throw 'Artifact SHA256 mismatch; nothing installed.' }
    $installHelp = & $binary service install --help | Out-String
    if ($LASTEXITCODE -ne 0 -or $installHelp -notmatch '(?m)^\s+--rebind\b') {
        throw "Runner $($lines[0]) does not support connection rebinding. Update the Ferry server and copy a new connection command from the updated page. Nothing installed."
    }
    if ($User -and $installHelp -notmatch 'Windows logon task') {
        throw "Runner $($lines[0]) does not support current-user installation on Windows. Update the Ferry server, or install as a system service. Nothing installed."
    }
    $scope = if ($User) { @('--user') } else { @() }
    Write-Host "Installing Runner $($lines[0]) as ${Name}$(if ($User) { ' for the current user' })..."
    # The token remains in the inherited environment, never the process argv.
    & $binary service install --server $Server --name $Name --rebind @scope
    if ($LASTEXITCODE -ne 0) { throw "Native service installation failed (exit $LASTEXITCODE)." }
    if ($User) { Write-Host "Runner ${Name} installed for $($identity.Name); it starts at logon and runs while you are logged on. Check the host connection status in Ferry." }
    else { Write-Host "Runner service ${Name} installed. Check the host connection status in Ferry." }
} finally {
    if ($client) { $client.Dispose() }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
