[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateSet('install', 'rollback', 'recover')][string]$Action,
    [Parameter(Mandatory = $true)][string]$Destination
)
$ErrorActionPreference = 'Stop'
$Destination = [IO.Path]::GetFullPath($Destination)
if ($Destination -eq [IO.Path]::GetPathRoot($Destination)) { throw 'The destination cannot be a filesystem root.' }
$Destination = $Destination.TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
$Parent = Split-Path -Parent $Destination
$History = "$Destination-rollbacks"
if (-not (Test-Path -LiteralPath $Parent -PathType Container)) {
    throw 'The destination parent directory must already exist.'
}
function Assert-OrdinaryPath([string]$Path) {
    for ($Current = $Path; $Current; $Current = Split-Path -Parent $Current) {
        $Item = Get-Item -LiteralPath $Current -Force -ErrorAction SilentlyContinue
        if ($Item -and ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Installation path must not cross a reparse point: $Current"
        }
        $Next = Split-Path -Parent $Current
        if ($Next -eq $Current) { break }
    }
}
Assert-OrdinaryPath $Destination
Assert-OrdinaryPath $History
$Journal = Join-Path $History 'pending.json'
Assert-OrdinaryPath $Journal
function Assert-Generation([string]$Generation) {
    if ([IO.Path]::GetFullPath($Generation) -ne $Generation -or
        (Split-Path -Parent $Generation) -ne $History -or
        (Split-Path -Leaf $Generation) -notmatch '^generation\.[0-9a-f]{32}$') {
        throw 'Invalid rollback generation.'
    }
    foreach ($Path in @($Generation, (Join-Path $Generation 'previous'),
        (Join-Path $Generation 'candidate'), (Join-Path $Generation 'retired'),
        (Join-Path $Generation 'failed-candidate'), (Join-Path $Generation 'committed'))) {
        Assert-OrdinaryPath $Path
    }
}
function Write-Journal([string]$Operation, [string]$Generation) {
    $Bytes = [Text.Encoding]::UTF8.GetBytes((@{ version = 1; action = $Operation; generation = $Generation } | ConvertTo-Json))
    $Temporary = Join-Path $History ('pending.' + [Guid]::NewGuid().ToString('N'))
    $Stream = [IO.File]::Open($Temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $Stream.Write($Bytes, 0, $Bytes.Length); $Stream.Flush($true) } finally { $Stream.Dispose() }
    [IO.File]::Move($Temporary, $Journal)
}
function Assert-Candidate([string]$Path, [string]$Generation) {
    Assert-OrdinaryPath $Path
    $Record = Join-Path $Path '.autopkg-rust-rollback'
    Assert-OrdinaryPath $Record
    if (-not (Test-Path -LiteralPath $Record -PathType Leaf) -or
        (Get-Content -LiteralPath $Record -Raw).Trim() -ne $Generation) {
        throw 'Interrupted transaction candidate does not match its generation.'
    }
}
# The persistent journal remains after process termination. An exclusive handle
# serializes installer invocations and is released by the OS even after a kill.
New-Item -ItemType Directory -Path $History -Force | Out-Null
$LockPath = Join-Path $History 'transaction.lock'
Assert-OrdinaryPath $LockPath
$Lock = [IO.File]::Open($LockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
try {
if (Test-Path -LiteralPath $Journal) {
    if ($Action -ne 'recover') { throw "Interrupted transaction: run the extracted install.ps1 recover -Destination '$Destination'." }
    $Pending = Get-Content -LiteralPath $Journal -Raw | ConvertFrom-Json
    if ($Pending.version -ne 1 -or $Pending.action -notin @('install', 'rollback')) { throw 'Invalid transaction journal.' }
    $Generation = [string]$Pending.generation
    Assert-Generation $Generation
    $Previous = Join-Path $Generation 'previous'
    $Candidate = Join-Path $Generation 'candidate'
    $Retired = Join-Path $Generation 'retired'
    if ($Pending.action -eq 'install') {
        if (Test-Path -LiteralPath (Join-Path $Generation 'committed')) {
            Assert-Candidate $Destination $Generation
        } else {
            $Failed = Join-Path $Generation 'failed-candidate'
            if (Test-Path -LiteralPath $Failed) {
                Assert-Candidate $Failed $Generation
            } elseif (-not (Test-Path -LiteralPath $Candidate) -and (Test-Path -LiteralPath $Destination)) {
                Assert-Candidate $Destination $Generation
                Move-Item -LiteralPath $Destination -Destination (Join-Path $Generation 'failed-candidate')
            }
            if (Test-Path -LiteralPath $Previous) {
                if (Test-Path -LiteralPath $Destination) { throw 'Recovery destination is occupied; preserving both generations.' }
                Move-Item -LiteralPath $Previous -Destination $Destination
            }
        }
    } else {
        if (-not (Test-Path -LiteralPath (Join-Path $Generation 'committed') -PathType Leaf)) { throw 'Rollback generation was not committed.' }
        if (Test-Path -LiteralPath $Retired) {
            Assert-Candidate $Retired $Generation
            if (Test-Path -LiteralPath $Previous) {
                if (Test-Path -LiteralPath $Destination) { throw 'Recovery destination is occupied; preserving both generations.' }
                Move-Item -LiteralPath $Previous -Destination $Destination
            }
        } else {
            Assert-Candidate $Destination $Generation
            Move-Item -LiteralPath $Destination -Destination $Retired
            if (Test-Path -LiteralPath $Previous) { Move-Item -LiteralPath $Previous -Destination $Destination }
        }
    }
    Remove-Item -LiteralPath $Journal
    Write-Output "Recovered interrupted $($Pending.action). Generations retained at $History"
    exit 0
}
if ($Action -eq 'recover') { Write-Output 'No interrupted transaction.'; exit 0 }
$Marker = Join-Path $Destination '.autopkg-rust-rollback'
if ($Action -eq 'rollback') {
    if (-not (Test-Path -LiteralPath $Marker -PathType Leaf)) { throw 'No native rollback record.' }
    Assert-OrdinaryPath $Marker
    $Generation = (Get-Content -LiteralPath $Marker -Raw).Trim()
    Assert-Generation $Generation
    if (-not (Test-Path -LiteralPath (Join-Path $Generation 'committed') -PathType Leaf)) {
        throw 'Rollback generation was not committed.'
    }
    $Retired = Join-Path $Generation 'retired'
    if (Test-Path -LiteralPath $Retired) { throw 'Rollback generation was already used.' }
    Write-Journal 'rollback' $Generation
    Move-Item -LiteralPath $Destination -Destination $Retired
    $Previous = Join-Path $Generation 'previous'
    try {
        if (Test-Path -LiteralPath $Previous) {
            Move-Item -LiteralPath $Previous -Destination $Destination
        }
    } catch {
        Move-Item -LiteralPath $Retired -Destination $Destination
        Remove-Item -LiteralPath $Journal
        throw
    }
    Remove-Item -LiteralPath $Journal
    Write-Output "Restored previous installation. Retired native files: $Retired"
    exit 0
}
$Binary = Join-Path $PSScriptRoot 'bin/autopkg-rs.exe'
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) { throw 'Missing native executable.' }
Assert-OrdinaryPath $Binary
New-Item -ItemType Directory -Path $History -Force | Out-Null
$Generation = Join-Path $History ("generation." + [Guid]::NewGuid().ToString('N'))
$Candidate = Join-Path $Generation 'candidate'
New-Item -ItemType Directory -Path $Candidate | Out-Null
Copy-Item -LiteralPath $Binary -Destination (Join-Path $Candidate 'autopkg.exe')
Copy-Item -LiteralPath $PSCommandPath -Destination (Join-Path $Candidate 'install.ps1')
Set-Content -LiteralPath (Join-Path $Candidate '.autopkg-rust-rollback') -Value $Generation -Encoding UTF8
$Previous = Join-Path $Generation 'previous'
Write-Journal 'install' $Generation
$Saved = $false
$Installed = $false
try {
    if (Test-Path -LiteralPath $Destination) {
        Move-Item -LiteralPath $Destination -Destination $Previous
        $Saved = $true
    }
    Move-Item -LiteralPath $Candidate -Destination $Destination
    $Installed = $true
    New-Item -ItemType File -Path (Join-Path $Generation 'committed') | Out-Null
} catch {
    if ($Installed) {
        Move-Item -LiteralPath $Destination -Destination (Join-Path $Generation 'failed-candidate')
    }
    if ($Saved) { Move-Item -LiteralPath $Previous -Destination $Destination }
    Remove-Item -LiteralPath $Journal
    throw
}
Remove-Item -LiteralPath $Journal
Write-Output "Installed $Destination/autopkg.exe. Previous installation retained at $Previous"
Write-Output "Rollback: & '$Destination/install.ps1' rollback -Destination '$Destination'"

} finally { $Lock.Dispose() }
