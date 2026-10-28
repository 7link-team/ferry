param(
    [Parameter(Mandatory = $true)][string]$Server,
    [string]$Name = 'runner-agent'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Open PowerShell as Administrator and run the installation command again.'
}
$serverUri = [Uri]$Server
if (-not $serverUri.IsAbsoluteUri -or $serverUri.Scheme -notin @('http', 'https') -or $serverUri.UserInfo -or $serverUri.Query -or $serverUri.Fragment) { throw 'Server must be an HTTP(S) URL without credentials, query or fragment.' }
if ($Name -cnotmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]{0,79}$') { throw 'Invalid service name.' }
if (-not $env:FERRY_TOKEN -or $env:FERRY_TOKEN -cnotmatch '^[a-zA-Z0-9._~-]+$') { throw 'Set FERRY_TOKEN to the enrolment token first.' }
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64') { throw 'Only Windows x64 is supported.' }

# Never stage a privileged executable in a user-writable temporary directory.
$admins = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
$system = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$trustedInstaller = ([Security.Principal.NTAccount]::new('NT SERVICE\TrustedInstaller')).Translate([Security.Principal.SecurityIdentifier]).Value
$trusted = @($admins.Value, $system.Value, $identity.User.Value, $trustedInstaller)
$replace = [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership
$write = $replace -bor [Security.AccessControl.FileSystemRights]::Write
$base = Get-Item -LiteralPath ([Environment]::GetFolderPath('ProgramFiles')) -Force
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
    $acl.SetOwner($admins)
    foreach ($sid in @($admins, $system)) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit, ObjectInherit', 'None', 'Allow'))
    }
    Set-Acl -LiteralPath $stage -AclObject $acl
    Add-Type -AssemblyName System.Net.Http
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(300)
    function Save-Download([Uri]$Url, [string]$Path, [long]$Limit, [bool]$Metadata) {
        $deadline = [Threading.CancellationTokenSource]::new([TimeSpan]::FromSeconds($(if ($Metadata) { 60 } else { 300 })))
        try {
            for ($redirect = 0; $redirect -le 5; $redirect++) {
                if (-not $Metadata -and $Url.Scheme -ne 'https') { throw 'Artifact URL and redirects must use HTTPS.' }
                $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::Get, $Url)
                $response = $null
                try {
                    if ($Metadata) { $request.Headers.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $env:FERRY_TOKEN) }
                    $response = $client.SendAsync($request, [Net.Http.HttpCompletionOption]::ResponseHeadersRead, $deadline.Token).GetAwaiter().GetResult()
                    if ([int]$response.StatusCode -ge 300 -and [int]$response.StatusCode -lt 400) {
                        if ($Metadata -or $redirect -eq 5 -or -not $response.Headers.Location) { throw 'Unexpected download redirect.' }
                        $Url = [Uri]::new($Url, $response.Headers.Location)
                        continue
                    }
                    $null = $response.EnsureSuccessStatusCode()
                    if ($response.Content.Headers.ContentLength -gt $Limit) { throw 'Download exceeds size limit.' }
                    $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
                    $outputStream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                    try {
                        $buffer = [byte[]]::new(65536)
                        [long]$total = 0
                        while (($count = $inputStream.ReadAsync($buffer, 0, $buffer.Length, $deadline.Token).GetAwaiter().GetResult()) -gt 0) {
                            $total += $count
                            if ($total -gt $Limit) { throw 'Download exceeds size limit.' }
                            $outputStream.Write($buffer, 0, $count)
                        }
                    } finally { $outputStream.Dispose(); $inputStream.Dispose() }
                    return
                } finally { if ($response) { $response.Dispose() }; $request.Dispose() }
            }
        } finally { $deadline.Dispose() }
    }
    $metadataPath = Join-Path $stage 'metadata'
    Save-Download ($Server.TrimEnd('/') + '/api/runners/install/x86_64-pc-windows-msvc') $metadataPath 8192 $true
    $metadata = [IO.File]::ReadAllText($metadataPath)
    $lines = $metadata.Split([char]10)
    if ($lines.Length -ne 5 -or $lines[4] -ne '' -or $lines[0] -cnotmatch '^[a-zA-Z0-9.+-]+$' -or $lines[2] -cnotmatch '^[0-9a-f]{64}$' -or $lines[3] -cnotmatch '^[1-9][0-9]{0,8}$') { throw 'Invalid installation metadata.' }
    $url = [Uri]$lines[1]
    if (-not $url.IsAbsoluteUri -or $url.Scheme -ne 'https') { throw 'Artifact URL must use HTTPS.' }
    $size = [long]$lines[3]
    if ($size -gt 268435456) { throw 'Artifact exceeds size limit.' }
    $binary = Join-Path $stage 'runner.exe'
    Save-Download $url $binary $size $false
    if ((Get-Item -LiteralPath $binary).Length -ne $size) { throw 'Artifact size mismatch; nothing installed.' }
    if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -cne $lines[2]) { throw 'Artifact SHA256 mismatch; nothing installed.' }
    Write-Host "Installing Runner $($lines[0]) as $Name…"
    # The token remains in the inherited environment, never the process argv.
    & $binary service install --server $Server --name $Name
    if ($LASTEXITCODE -ne 0) { throw "Native service installation failed (exit $LASTEXITCODE)." }
} finally {
    if ($client) { $client.Dispose() }
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
