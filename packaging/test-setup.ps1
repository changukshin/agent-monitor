param([Parameter(Mandatory)][string]$Package)
$ErrorActionPreference='Stop'
# Run on a disposable CI account. Never replace another user's registration.
$link=Join-Path ([Environment]::GetFolderPath('Programs')) 'Agent Monitor Dashboard.lnk'
if (Test-Path -LiteralPath $link) {throw 'Test requires an unused shortcut name.'}
$temp=Join-Path $env:TEMP ('agent-monitor-'+[guid]::NewGuid())
$dir=Join-Path $temp ('Test '+[char]0xd55c)
$old=Join-Path $temp 'previous'
New-Item -ItemType Directory -Path $old -Force|Out-Null
Copy-Item -Path (Join-Path $Package '*') -Destination $old
function Run($pkg,$action,[bool]$success=$true) {
    & powershell.exe -NoProfile -File (Join-Path $pkg 'Setup.ps1') -Action $action -InstallRoot $dir
    if (($LASTEXITCODE -eq 0) -ne $success) {throw "Unexpected $action result: $LASTEXITCODE"}
}
# A predecessor fixture with a different collector hash exercises actual replacement.
$exe='agent-monitor-collector.exe'
[IO.File]::AppendAllText((Join-Path $old $exe),'old fixture')
$m=Get-Content (Join-Path $old 'build.json') -Raw|ConvertFrom-Json
$m.binaries.$exe=(Get-FileHash (Join-Path $old $exe)).Hash
$m|ConvertTo-Json -Depth 5|Set-Content (Join-Path $old 'build.json')
Run $old Install
$config=Join-Path $dir 'config.json'
[IO.File]::AppendAllText($config," ")
$before=(Get-FileHash $config).Hash
$guard=[IO.File]::Open((Join-Path $dir $exe),'Open','Read','Read')
try {Run $Package Install $false} finally {$guard.Dispose()}
Run $Package Install
Run $Package Install
if ((Get-FileHash (Join-Path $dir $exe)).Hash -ne (Get-FileHash (Join-Path $Package $exe)).Hash) {throw 'Upgrade payload mismatch.'}
$extra=Join-Path $dir 'personal.txt'
[IO.File]::WriteAllText($extra,'keep')
$original=[IO.File]::ReadAllBytes($link)
[IO.File]::AppendAllText($link,'edited')
Run $Package Uninstall $false
if (!(Test-Path $link)) {throw 'Edited shortcut was removed.'}
[IO.File]::WriteAllBytes($link,$original)
Run $Package Uninstall
Run $Package Uninstall
if ((Test-Path $link) -or (Test-Path (Join-Path $dir $exe)) -or !(Test-Path $extra) -or (Get-FileHash $config).Hash -ne $before) {throw 'Uninstall preservation failed.'}
Write-Output 'PASS: Unicode path, upgrade, in-use refusal, repeat install/removal and preservation.'
