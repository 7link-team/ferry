$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
# A current-user installation keeps its state private to that user (plus
# SYSTEM), as a 0700 directory does on Unix; it needs no elevation.
$userScope = $env:RUNNER_SERVICE_SCOPE -eq 'user'
if (-not $userScope -and -not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Service installation requires administrator privileges' }
$system = [Security.Principal.SecurityIdentifier]::new('S-1-5-18')
$admins = [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
$owner = if ($userScope) { $identity.User } else { $admins }
$grantees = if ($userScope) { @($identity.User, $system) } else { @($system, $admins) }
$trustedInstaller = ([Security.Principal.NTAccount]::new('NT SERVICE\TrustedInstaller')).Translate([Security.Principal.SecurityIdentifier]).Value
$trusted = @($system.Value, $admins.Value, $identity.User.Value, $trustedInstaller)
$replaceRights = [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership
$writeRights = $replaceRights -bor [Security.AccessControl.FileSystemRights]::WriteData -bor [Security.AccessControl.FileSystemRights]::AppendData -bor [Security.AccessControl.FileSystemRights]::WriteAttributes -bor [Security.AccessControl.FileSystemRights]::WriteExtendedAttributes
function Assert-Secure($item, $mask) {
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse points are not allowed for service state: $($item.FullName)" }
    $acl = Get-Acl -LiteralPath $item.FullName
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trusted) { throw "Untrusted service path owner: $($item.FullName)" }
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) { continue }
        # OWNER RIGHTS names this object's owner, already verified above. It is
        # not a separate untrusted account (Python-created directories use it).
        if ($rule.IdentityReference.Value -eq 'S-1-3-4') { continue }
        if ($rule.AccessControlType -eq [Security.AccessControl.AccessControlType]::Allow -and $rule.IdentityReference.Value -notin $trusted -and ($rule.FileSystemRights -band $mask)) { throw "Untrusted write access to service path: $($item.FullName); principal=$($rule.IdentityReference.Value); rights=$($rule.FileSystemRights)" }
    }
}
# Validate existing ancestors before creating anything. ProgramData permits
# users to create children, so every missing component needs a protected ACL
# at creation, not a later Set-Acl on a directory with inherited user writes.
$missing = [Collections.Generic.List[string]]::new()
$cursor = [IO.DirectoryInfo]::new([IO.Path]::GetFullPath($env:RUNNER_SERVICE_STATE_DIRECTORY))
while (-not (Test-Path -LiteralPath $cursor.FullName)) {
    $missing.Insert(0, $cursor.FullName)
    $cursor = $cursor.Parent
    if (-not $cursor) { throw 'Service state has no existing ancestor' }
}
$ancestor = Get-Item -LiteralPath $cursor.FullName -Force
if (-not $ancestor.PSIsContainer) { throw 'Service state ancestor is not a directory' }
while ($ancestor) { Assert-Secure $ancestor $replaceRights; $ancestor = $ancestor.Parent }
$creationAcl = [Security.AccessControl.DirectorySecurity]::new()
$creationAcl.SetAccessRuleProtection($true, $false)
$creationAcl.SetOwner($owner)
foreach ($sid in $grantees) {
    $creationAcl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, [Security.AccessControl.FileSystemRights]::FullControl, [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit', [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow))
}
foreach ($path in $missing) {
    # .NET Framework's overload supplies the security descriptor to creation.
    # If another process created the path first, it does not repair its ACL;
    # the following validation rejects an unsafe object before any writes.
    $null = [IO.Directory]::CreateDirectory($path, $creationAcl)
    $created = Get-Item -LiteralPath $path -Force
    Assert-Secure $created $writeRights
}
$root = Get-Item -LiteralPath $env:RUNNER_SERVICE_STATE_DIRECTORY -Force
if (-not $root.PSIsContainer) { throw 'Service state is not a directory' }
Assert-Secure $root $writeRights
$parent = $root.Parent
while ($parent) { Assert-Secure $parent $replaceRights; $parent = $parent.Parent }
# Reject existing files with unsafe grants before changing permissions; do not
# promote previously untrusted executables or credentials into a system service.
$children = @(Get-ChildItem -LiteralPath $root.FullName -Recurse -Force)
foreach ($child in $children) { Assert-Secure $child $writeRights }
foreach ($item in @($root) + $children) {
    if ($item.PSIsContainer) {
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    } else {
        $acl = [Security.AccessControl.FileSecurity]::new()
        $inherit = [Security.AccessControl.InheritanceFlags]::None
    }
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner($owner)
    foreach ($sid in $grantees) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, [Security.AccessControl.FileSystemRights]::FullControl, $inherit, [Security.AccessControl.PropagationFlags]::None, [Security.AccessControl.AccessControlType]::Allow))
    }
    # Set-Acl also rewrites the audit SACL, which needs SeSecurityPrivilege.
    # An unprivileged user writes only the owner and DACL sections it changed.
    if ($userScope) { $item.SetAccessControl($acl) } else { Set-Acl -LiteralPath $item.FullName -AclObject $acl }
}
