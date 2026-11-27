#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('Install','Uninstall')][string]$Action='Install',
    [string]$InstallRoot=(Join-Path $env:LOCALAPPDATA 'Agent Monitor')
)
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
if ($PSVersionTable.PSEdition -ne 'Desktop') { throw 'Run with Windows PowerShell (powershell.exe).' }
. (Join-Path $PSScriptRoot 'Shortcut.ps1')
$root=[IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
if ($root -notmatch '^[A-Za-z]:\\.+' -or $root.Contains('"')) {throw 'Select a local application directory.'}
$programs=[Environment]::GetFolderPath('Programs')
$linkPath=Join-Path $programs 'Agent Monitor Dashboard.lnk'
$receipt=Join-Path $root 'installation.json'
$api=Join-Path $root 'api'
$config=Join-Path $root 'config.json'
$names=@('agent-monitor-collector.exe','agent-monitor-dashboard.exe')
$dashboard=Join-Path $root $names[1]
$arguments='32941 "'+$api+'"'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User
function Assert-Plain($path) {
    while ($path) {
        $item=Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {throw 'Reparse points are not supported in installation paths.'}
        $path=[IO.Path]::GetDirectoryName($path)
    }
}
function Assert-Private($path) {
    $acl=Get-Acl -LiteralPath $path
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]) -ne $sid) {throw 'Unexpected file owner.'}
    $allowed=@($sid.Value,'S-1-5-18','S-1-5-32-544')
    foreach ($rule in $acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])) {
        if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -notin $allowed) {throw 'Installation path is not private.'}
    }
}
function New-Private($path) {
    $acl=New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true,$false)
    $acl.SetOwner($sid)
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')))
    [void][IO.Directory]::CreateDirectory($path,$acl)
    Assert-Private $path
}
function Hash($path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
function Publish-File($temp,$dest) {
    if ([IO.File]::Exists($dest)) {
        # File.Replace requires a real backup path in Windows PowerShell.
        $backup=$temp+'.previous'
        [IO.File]::Replace($temp,$dest,$backup)
        [IO.File]::Delete($backup)
    } else { [IO.File]::Move($temp,$dest) }
}
# Elevated Windows build accounts can default new files to Administrators.
# Assign the operator SID only on a freshly created payload before publication.
function Set-NewFileOwner($path) {
    $acl=Get-Acl -LiteralPath $path
    $acl.SetOwner($sid)
    Set-Acl -LiteralPath $path -AclObject $acl
    Assert-Private $path
}
function Save-Receipt($state) {
    $temp=Join-Path $root ([guid]::NewGuid().ToString()+'.tmp')
    [IO.File]::WriteAllText($temp,($state|ConvertTo-Json -Depth 5))
    Set-NewFileOwner $temp
    Publish-File $temp $receipt
}
$mutex=New-Object Threading.Mutex($false,('Local\AgentMonitorSetup-'+$sid.Value))
try { $locked=$mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] {$locked=$true}
if (!$locked) {throw 'Another installation operation is active.'}
try {
    foreach ($path in @($root,$receipt,$api,$config,$linkPath)+( $names | ForEach-Object {Join-Path $root $_} )) {Assert-Plain $path}
    $present=[IO.File]::Exists($receipt)
    if (!$present -and $Action -eq 'Uninstall') {Write-Output 'No owned installation.';return}
    if (!$present -and ((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $linkPath))) {throw 'Installation directory or shortcut already exists without an ownership receipt.'}
    if ($present) {
        Assert-Private $root
        Assert-Private $receipt
        $state=Get-Content -Encoding UTF8 -Raw -LiteralPath $receipt|ConvertFrom-Json
        if ($state.product -cne 'AgentMonitor.Dashboard' -or $state.root -ne $root) {throw 'Invalid ownership receipt.'}
    } else {
        New-Private $root
        $state=[pscustomobject]@{product='AgentMonitor.Dashboard';root=$root;hashes=[pscustomobject]@{};shortcut=@()}
        Save-Receipt $state
    }
    foreach ($name in $names) {
        $path=Join-Path $root $name
        if (Test-Path -LiteralPath $path) {
            $entry=$state.hashes.PSObject.Properties[$name]
            if (!$entry -or (Hash $path) -notin $entry.Value) {throw "Preserving unrecognized or edited file: $name"}
            $handle=[IO.File]::Open($path,'Open','ReadWrite','None')
            $handle.Dispose()
        }
    }
    if (Test-Path -LiteralPath $linkPath) {
        if ((Hash $linkPath) -notin $state.shortcut) {throw 'Preserving an unowned or modified shortcut.'}
        Assert-DashboardLink $linkPath $dashboard $root $arguments
    }
    if (Test-Path -LiteralPath $api) {Assert-Private $api}
    if ($Action -eq 'Uninstall') {
        foreach ($name in $names) {[IO.File]::Delete((Join-Path $root $name))}
        [IO.File]::Delete($linkPath)
        # Keep user-managed data and the ownership receipt for safe reinstall.
        Write-Output 'Removed owned application files and shortcut. Private data retained.'
        return
    }
    if (!(Test-Path -LiteralPath $api)) {New-Private $api}
    if (!(Test-Path -LiteralPath $config)) {
        $example=Get-Content -Encoding UTF8 -Raw -LiteralPath (Join-Path $PSScriptRoot 'config.example.json')|ConvertFrom-Json
        $example.private_directory=$api
        $temp=Join-Path $root ([guid]::NewGuid().ToString()+'.tmp')
        [IO.File]::WriteAllText($temp,($example|ConvertTo-Json -Depth 6))
        Set-NewFileOwner $temp
        [IO.File]::Move($temp,$config)
    }
    Assert-Private $config
    $manifest=Get-Content -Encoding UTF8 -Raw -LiteralPath (Join-Path $PSScriptRoot 'build.json')|ConvertFrom-Json
    foreach ($name in $names) {
        $src=Join-Path $PSScriptRoot $name
        $dest=Join-Path $root $name
        $desired=Hash $src
        if ($desired -ne $manifest.binaries.$name) {throw "Package checksum mismatch: $name"}
        $old=if ([IO.File]::Exists($dest)) {Hash $dest}
        if ($old -ne $desired) {
            $state.hashes|Add-Member -Force NoteProperty $name @($old,$desired)
            Save-Receipt $state
            $temp=Join-Path $root ([guid]::NewGuid().ToString()+'.tmp')
            [IO.File]::Copy($src,$temp)
            if ((Hash $temp) -ne $desired) {throw 'Staged binary checksum mismatch.'}
            Publish-File $temp $dest
        }
        $state.hashes|Add-Member -Force NoteProperty $name @($desired)
    }
    if (!(Test-Path -LiteralPath $linkPath)) {
        $temp=Join-Path $programs ([guid]::NewGuid().ToString()+'.lnk')
        try {
            New-DashboardLink $temp $dashboard $root $arguments
            Assert-DashboardLink $temp $dashboard $root $arguments
            $state.shortcut=@((Hash $temp))
            Save-Receipt $state
            [IO.File]::Move($temp,$linkPath)
        } finally {if ([IO.File]::Exists($temp)) {[IO.File]::Delete($temp)}}
    }
    Assert-DashboardLink $linkPath $dashboard $root $arguments
    Save-Receipt $state
    Write-Output "Installed and verified. Configuration: $config"
} finally {$mutex.ReleaseMutex();$mutex.Dispose()}
