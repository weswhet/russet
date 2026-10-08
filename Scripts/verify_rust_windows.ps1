#requires -Version 7.2
<#
.SYNOPSIS
Validate native Windows processors without installing the Chocolatey package.
.DESCRIPTION
Requires Windows x64, Chocolatey, and Windows SDK signtool. Accepts either the
unsigned development CLI or its ZIP archive. All fixture files, preferences,
and AutoPkg caches live under one temporary directory. Missing prerequisites
fail the gate instead of silently skipping native coverage.
#>
[CmdletBinding(DefaultParameterSetName = 'Binary')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'Binary')]
    [Alias('Executable')][string]$Binary,
    [Parameter(Mandatory = $true, ParameterSetName = 'Archive')]
    [string]$ArchivePath,
    [string]$ChocolateyPath,
    [string]$SignToolPath,
    [switch]$KeepArtifacts
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'This integration gate requires native Windows x64.'
}

function Assert-That([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-NoPython {
    foreach ($Name in @('python', 'python3', 'py')) {
        $Found = @(Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue)
        Assert-That ($Found.Count -eq 0) "Python is discoverable during the native gate: $Name"
    }
}

function Write-PlistValue([Xml.XmlWriter]$Writer, $Value) {
    if ($Value -is [Collections.IDictionary]) {
        $Writer.WriteStartElement('dict')
        foreach ($Key in $Value.Keys) {
            $Writer.WriteElementString('key', [string]$Key)
            Write-PlistValue $Writer $Value[$Key]
        }
        $Writer.WriteEndElement()
    } elseif ($Value -is [bool]) {
        $Writer.WriteStartElement($(if ($Value) { 'true' } else { 'false' }))
        $Writer.WriteEndElement()
    } elseif ($Value -is [int] -or $Value -is [long]) {
        $Writer.WriteElementString('integer', $Value.ToString([Globalization.CultureInfo]::InvariantCulture))
    } elseif ($Value -is [string]) {
        $Writer.WriteElementString('string', $Value)
    } elseif ($Value -is [Collections.IEnumerable]) {
        $Writer.WriteStartElement('array')
        foreach ($Item in $Value) { Write-PlistValue $Writer $Item }
        $Writer.WriteEndElement()
    } else {
        throw 'Unsupported integration fixture plist value.'
    }
}

function ConvertTo-Plist($Value) {
    $Builder = [Text.StringBuilder]::new()
    $Settings = [Xml.XmlWriterSettings]::new()
    $Settings.OmitXmlDeclaration = $true
    $Writer = [Xml.XmlWriter]::Create($Builder, $Settings)
    try {
        $Writer.WriteStartElement('plist')
        $Writer.WriteAttributeString('version', '1.0')
        Write-PlistValue $Writer $Value
        $Writer.WriteEndElement()
        $Writer.Flush()
        return $Builder.ToString()
    } finally { $Writer.Dispose() }
}

function Get-PlistNode([xml]$Document, [string]$Key) {
    # Keys here are fixed fixture identifiers, never user-provided XPath.
    $Node = $Document.SelectSingleNode("/plist/dict/key[text()='$Key']/following-sibling::*[1]")
    if ($null -eq $Node) { throw "Processor output is missing $Key" }
    return $Node
}

function Invoke-Native([string]$Program, [string[]]$Arguments, [string]$InputText = '') {
    Assert-NoPython
    $Info = [Diagnostics.ProcessStartInfo]::new()
    $Info.FileName = $Program
    $Info.UseShellExecute = $false
    $Info.RedirectStandardInput = $true
    $Info.RedirectStandardOutput = $true
    $Info.RedirectStandardError = $true
    $Info.StandardInputEncoding = [Text.UTF8Encoding]::new($false)
    $Info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $Info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $Info.WorkingDirectory = $script:FixtureRoot
    foreach ($Argument in $Arguments) { $Info.ArgumentList.Add($Argument) }
    $Process = [Diagnostics.Process]::new()
    $Process.StartInfo = $Info
    try {
        [void]$Process.Start()
        $OutputTask = $Process.StandardOutput.ReadToEndAsync()
        $ErrorTask = $Process.StandardError.ReadToEndAsync()
        $Process.StandardInput.Write($InputText)
        $Process.StandardInput.Close()
        if (-not $Process.WaitForExit(300000)) {
            $Process.Kill($true)
            throw "Native command timed out: $Program $($Arguments -join ' ')"
        }
        return [pscustomobject]@{
            ExitCode = $Process.ExitCode
            Stdout = $OutputTask.GetAwaiter().GetResult()
            Stderr = $ErrorTask.GetAwaiter().GetResult()
        }
    } finally { $Process.Dispose() }
}

function Invoke-Processor([string]$Name, [Collections.IDictionary]$Values, [switch]$ExpectFailure) {
    $Result = Invoke-Native $script:NativeBinary @('processor-run', $Name) (ConvertTo-Plist $Values)
    if ($ExpectFailure) {
        Assert-That ($Result.ExitCode -ne 0) "$Name unexpectedly succeeded."
        return $Result
    }
    Assert-That ($Result.ExitCode -eq 0) "$Name failed ($($Result.ExitCode)): $($Result.Stderr)"
    Write-Verbose $Result.Stderr
    return [xml]$Result.Stdout
}

function Read-ZipEntry([IO.Compression.ZipArchiveEntry]$Entry) {
    Assert-That ($null -ne $Entry) 'Expected ZIP entry is absent.'
    $InputStream = $Entry.Open()
    $OutputStream = [IO.MemoryStream]::new()
    try {
        $InputStream.CopyTo($OutputStream)
        return ,$OutputStream.ToArray()
    } finally { $InputStream.Dispose(); $OutputStream.Dispose() }
}

function Test-ArchiveInstallation([string]$PackageBinary) {
    $PackageRoot = Split-Path -Parent (Split-Path -Parent $PackageBinary)
    $Installer = Join-Path $PackageRoot 'install.ps1'
    Assert-That (Test-Path -LiteralPath $Installer -PathType Leaf) 'Archive is missing install.ps1.'
    $Destination = Join-Path $script:FixtureRoot 'installed package'
    [void][IO.Directory]::CreateDirectory($Destination)
    $LegacyBinary = Join-Path $Destination 'autopkg.exe'
    $InstalledBinary = Join-Path $Destination 'russet.exe'
    $LegacyPreferences = Join-Path $Destination 'preferences.dat'
    [IO.File]::WriteAllBytes($LegacyBinary, [byte[]]@(0, 1, 255, 13, 10))
    [IO.File]::WriteAllText($LegacyPreferences, 'existing fixture preferences')
    $LegacyHash = (Get-FileHash -LiteralPath $LegacyBinary -Algorithm SHA256).Hash
    $PreferencesHash = (Get-FileHash -LiteralPath $LegacyPreferences -Algorithm SHA256).Hash
    $PackageHash = (Get-FileHash -LiteralPath $PackageBinary -Algorithm SHA256).Hash
    $PowerShell = Join-Path $PSHOME 'pwsh.exe'
    $FirstMarker = $null
    foreach ($Action in @('install', 'install', 'rollback', 'rollback')) {
        # Run a separate process because the packaged installer exits on rollback.
        $Result = Invoke-Native $PowerShell @('-NoProfile', '-NonInteractive', '-File', $Installer,
            '-Action', $Action, '-Destination', $Destination)
        Assert-That ($Result.ExitCode -eq 0) "Packaged $Action failed: $($Result.Stderr)"
        $Marker = Join-Path $Destination '.russet-rollback'
        if ($Action -eq 'install') {
            Assert-That ((Get-FileHash -LiteralPath $InstalledBinary -Algorithm SHA256).Hash -eq $PackageHash) 'Installed binary differs from archive.'
            $CurrentMarker = [IO.File]::ReadAllText($Marker)
            if ($null -eq $FirstMarker) { $FirstMarker = $CurrentMarker }
            else { Assert-That ($CurrentMarker -ne $FirstMarker) 'Upgrade did not create a separate rollback generation.' }
            $Version = Invoke-Native $InstalledBinary @('version')
            Assert-That ($Version.ExitCode -eq 0) "Installed CLI cannot execute: $($Version.Stderr)"
        } elseif (Test-Path -LiteralPath $Marker) {
            Assert-That ([IO.File]::ReadAllText($Marker) -eq $FirstMarker) 'First rollback did not restore the first installation record.'
            Assert-That ((Get-FileHash -LiteralPath $InstalledBinary -Algorithm SHA256).Hash -eq $PackageHash) 'First rollback changed the previous binary.'
        } else {
            Assert-That ((Get-FileHash -LiteralPath $LegacyBinary -Algorithm SHA256).Hash -eq $LegacyHash) 'Second rollback did not restore original binary bytes.'
            Assert-That ((Get-FileHash -LiteralPath $LegacyPreferences -Algorithm SHA256).Hash -eq $PreferencesHash) 'Second rollback did not restore original preferences.'
            Assert-That (@(Get-ChildItem -LiteralPath $Destination -Force).Count -eq 2) 'Second rollback left extra files in the original installation.'
        }
    }
    Assert-That (-not (Test-Path -LiteralPath (Join-Path $Destination '.russet-rollback'))) 'Two rollbacks did not exhaust the native generations.'
}

$script:FixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ('autopkg-windows-gate-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($FixtureRoot)
$SavedEnvironment = @{}
foreach ($Name in @('HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'XDG_CONFIG_HOME', 'TEMP', 'TMP', 'PATH')) {
    $SavedEnvironment[$Name] = [Environment]::GetEnvironmentVariable($Name, 'Process')
}
try {
    if ($PSCmdlet.ParameterSetName -eq 'Archive') {
        $ArchivePath = (Resolve-Path -LiteralPath $ArchivePath).Path
        $Expanded = Join-Path $FixtureRoot 'archive'
        [IO.Compression.ZipFile]::ExtractToDirectory($ArchivePath, $Expanded)
        $Candidates = @(Get-ChildItem -LiteralPath $Expanded -Filter 'russet.exe' -File -Recurse)
        Assert-That ($Candidates.Count -eq 1) 'Archive must contain exactly one russet.exe.'
        $Binary = $Candidates[0].FullName
    }
    $script:NativeBinary = (Resolve-Path -LiteralPath $Binary).Path
    if (-not $ChocolateyPath) {
        $ChocolateyPath = (Get-Command choco.exe -CommandType Application -ErrorAction Stop).Source
    }
    $ChocolateyPath = (Resolve-Path -LiteralPath $ChocolateyPath).Path
    if (-not $SignToolPath) {
        $Candidates = @()
        foreach ($ProgramRoot in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
            if (-not $ProgramRoot) { continue }
            $Kits = Join-Path $ProgramRoot 'Windows Kits/10'
            if (-not (Test-Path -LiteralPath $Kits)) { continue }
            $Candidates += @(Get-ChildItem -LiteralPath (Join-Path $Kits 'bin') -Filter signtool.exe -File -Recurse -ErrorAction SilentlyContinue |
                Where-Object { $_.Directory.Name -eq 'x64' } | Sort-Object FullName -Descending)
            $Fallback = Join-Path $Kits 'App Certification Kit/signtool.exe'
            if (Test-Path -LiteralPath $Fallback) { $Candidates += Get-Item -LiteralPath $Fallback }
        }
        Assert-That ($Candidates.Count -gt 0) 'Windows SDK signtool.exe was not found; install the SDK or pass -SignToolPath.'
        $SignToolPath = $Candidates[0].FullName
    }
    $SignToolPath = (Resolve-Path -LiteralPath $SignToolPath).Path
    $SystemDirectory = Join-Path $env:WINDIR 'System32'

    foreach ($Name in @('HOME', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'XDG_CONFIG_HOME', 'TEMP', 'TMP')) {
        $Directory = Join-Path $FixtureRoot $Name
        [void][IO.Directory]::CreateDirectory($Directory)
        [Environment]::SetEnvironmentVariable($Name, $Directory, 'Process')
    }
    $Cache = Join-Path $FixtureRoot 'cache'
    [void][IO.Directory]::CreateDirectory($Cache)
    $Prefs = Join-Path $FixtureRoot 'preferences.plist'
    [IO.File]::WriteAllText($Prefs, (ConvertTo-Plist @{
        CACHE_DIR = $Cache; RECIPE_SEARCH_DIRS = @(); RECIPE_OVERRIDE_DIRS = @();
        RECIPE_REPO_DIR = (Join-Path $FixtureRoot 'repos')
    }), [Text.UTF8Encoding]::new($false))

    # Resolve tools before restricting discovery. Keep only native prerequisites;
    # never restore the runner's Python-bearing PATH between workflow phases.
    $NativePath = @($SystemDirectory, (Join-Path $SystemDirectory 'WindowsPowerShell/v1.0'),
        $PSHOME, (Split-Path -Parent $ChocolateyPath), (Split-Path -Parent $SignToolPath)) |
        Select-Object -Unique
    [Environment]::SetEnvironmentVariable('PATH', ($NativePath -join [IO.Path]::PathSeparator), 'Process')
    Assert-NoPython

    if ($PSCmdlet.ParameterSetName -eq 'Archive') { Test-ArchiveInstallation $NativeBinary }

    # Portable execution must remain native even when no executable is on PATH.
    [Environment]::SetEnvironmentVariable('PATH', '', 'Process')
    try {
        $Created = Join-Path $FixtureRoot 'native-file.txt'
        [void](Invoke-Processor 'FileCreator' @{ file_path = $Created; file_content = 'native standalone' })
        Assert-That ([IO.File]::ReadAllText($Created) -eq 'native standalone') 'Standalone FileCreator output differs.'
        $Predicate = Invoke-Processor 'StopProcessingIf' @{ name = 'Café'; count = 7; predicate = 'name ==[cd] "CAFE" AND count == 7' }
        Assert-That ((Get-PlistNode $Predicate 'stop_processing_recipe').LocalName -eq 'true') 'Portable predicate did not match.'
        $PlistPath = Join-Path $FixtureRoot 'native.plist'
        $RecipePath = Join-Path $FixtureRoot 'native.recipe'
        $Sentinel = Join-Path $FixtureRoot 'must-not-exist.txt'
        $Recipe = @{
            Identifier = 'org.autopkg.windows.native-gate'; Input = @{ tags = @('alpha', 'beta') };
            Process = @(
                @{ Processor = 'PlistEditor'; Arguments = @{ output_plist_path = $PlistPath; plist_data = @{ Name = 'Café'; Count = 7; Enabled = $true } } },
                @{ Processor = 'PlistReader'; Arguments = @{ info_path = $PlistPath; plist_keys = @{ Name = 'name'; Count = 'count' } } },
                @{ Processor = 'StopProcessingIf'; Arguments = @{ predicate = 'name ==[cd] "CAFE" AND count == 7 AND "beta" IN tags' } },
                @{ Processor = 'FileCreator'; Arguments = @{ file_path = $Sentinel; file_content = 'stop phase failed' } }
            )
        }
        [IO.File]::WriteAllText($RecipePath, (ConvertTo-Plist $Recipe), [Text.UTF8Encoding]::new($false))
        $Run = Invoke-Native $NativeBinary @('run', '--prefs', $Prefs, $RecipePath)
        Assert-That ($Run.ExitCode -eq 0) "Native recipe failed: $($Run.Stderr)"
        Assert-That (-not (Test-Path -LiteralPath $Sentinel)) 'Predicate stop allowed a later recipe step.'
        $NativePlist = [xml][IO.File]::ReadAllText($PlistPath)
        Assert-That ((Get-PlistNode $NativePlist 'Count').LocalName -eq 'integer') 'Plist integer type changed.'
        Assert-That ((Get-PlistNode $NativePlist 'Enabled').LocalName -eq 'true') 'Plist boolean type changed.'
    } finally {
        [Environment]::SetEnvironmentVariable('PATH', ($NativePath -join [IO.Path]::PathSeparator), 'Process')
        Assert-NoPython
    }

    # Pack a ZIP payload; never execute chocolateyInstall.ps1 or choco install.
    $PayloadDirectory = Join-Path $FixtureRoot 'payload'
    [void][IO.Directory]::CreateDirectory($PayloadDirectory)
    [IO.File]::WriteAllText((Join-Path $PayloadDirectory 'fixture.txt'), 'AutoPkg native package fixture')
    $PayloadZip = Join-Path $FixtureRoot 'payload.zip'
    [IO.Compression.ZipFile]::CreateFromDirectory($PayloadDirectory, $PayloadZip)
    $PackageResult = Invoke-Processor 'ChocolateyPackager' @{
        id = 'autopkg-native-ci-fixture'; version = '1.0.0'; title = 'Native <&> fixture';
        authors = 'AutoPkg'; description = 'Isolated native integration fixture; never installed.';
        chocoexe_path = $ChocolateyPath; installer_path = $PayloadZip; installer_type = 'zip';
        RECIPE_CACHE_DIR = $Cache; KEEP_BUILD_DIRECTORY = $true;
        additional_install_actions = "# autopkg-native-windows-gate`n"
    }
    $PackagePath = (Get-PlistNode $PackageResult 'nuget_package_path').InnerText
    Assert-That ([IO.Path]::GetFullPath($PackagePath).StartsWith($FixtureRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) 'Package escaped the isolated fixture root.'
    $Package = [IO.Compression.ZipFile]::OpenRead($PackagePath)
    try {
        $Specifications = @($Package.Entries | Where-Object { $_.FullName.EndsWith('.nuspec') })
        Assert-That ($Specifications.Count -eq 1) 'NuGet package must have one nuspec.'
        $Specification = [Xml.XmlDocument]::new()
        $SpecificationStream = [IO.MemoryStream]::new((Read-ZipEntry $Specifications[0]), $false)
        try { $Specification.Load($SpecificationStream) }
        finally { $SpecificationStream.Dispose() }
        $Metadata = $Specification.SelectSingleNode('/*[local-name()="package"]/*[local-name()="metadata"]')
        Assert-That ($Metadata.SelectSingleNode('*[local-name()="id"]').InnerText -eq 'autopkg-native-ci-fixture') 'NuGet identifier differs.'
        Assert-That ($Metadata.SelectSingleNode('*[local-name()="version"]').InnerText -eq '1.0.0') 'NuGet version differs.'
        Assert-That ($Metadata.SelectSingleNode('*[local-name()="title"]').InnerText -eq 'Native <&> fixture') 'NuGet XML escaping differs.'
        $Script = [Text.Encoding]::UTF8.GetString((Read-ZipEntry ($Package.GetEntry('tools/chocolateyInstall.ps1'))))
        Assert-That ($Script.Contains('Get-ChocolateyUnzip @packageArgs') -and $Script.Contains('payload.zip') -and $Script.Contains('# autopkg-native-windows-gate')) 'Chocolatey installation script differs.'
        $Tokens = $null; $ParseErrors = $null
        [void][Management.Automation.Language.Parser]::ParseInput($Script, [ref]$Tokens, [ref]$ParseErrors)
        Assert-That ($ParseErrors.Count -eq 0) 'Generated Chocolatey script contains PowerShell syntax errors.'
        $Embedded = Read-ZipEntry ($Package.GetEntry('tools/payload.zip'))
        $ActualHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Embedded))
        $ExpectedHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([IO.File]::ReadAllBytes($PayloadZip)))
        Assert-That ($ActualHash -eq $ExpectedHash) 'Packaged ZIP payload bytes differ.'
        Assert-That ($null -ne $Package.GetEntry('tools/payload.zip.ignore')) 'Package is missing the shim suppression marker.'
    } finally { $Package.Dispose() }

    $SignedPath = $null
    foreach ($Relative in @('cmd.exe', 'where.exe', 'WindowsPowerShell/v1.0/powershell.exe')) {
        $Candidate = Join-Path $SystemDirectory $Relative
        if (-not (Test-Path -LiteralPath $Candidate)) { continue }
        $Signature = Get-AuthenticodeSignature -LiteralPath $Candidate
        if ($Signature.Status -ne 'Valid' -or $null -eq $Signature.SignerCertificate -or
            $Signature.SignerCertificate.Subject -notmatch 'Microsoft') { continue }
        $Probe = Invoke-Native $SignToolPath @('verify', '/v', '/pa', '/a', $Candidate)
        if ($Probe.ExitCode -eq 0) { $SignedPath = $Candidate; break }
    }
    Assert-That ($null -ne $SignedPath) 'No Microsoft-signed system executable passed real signtool verification.'
    [void](Invoke-Processor 'SignToolVerifier' @{ input_path = $SignedPath; signtool_path = $SignToolPath; additional_arguments = @('/a') })
    Assert-That ((Get-AuthenticodeSignature -LiteralPath $NativeBinary).Status -eq 'NotSigned') 'Negative signature fixture requires the unsigned development CLI.'
    $Rejected = Invoke-Processor 'SignToolVerifier' @{ input_path = $NativeBinary; signtool_path = $SignToolPath; additional_arguments = @('/a') } -ExpectFailure
    Assert-That ($Rejected.Stderr.Contains('Authenticode verification failed')) 'Unsigned CLI failed for an unexpected reason.'
    [ordered]@{
        native_windows = $true; native_workflow_python_absent = 'passed'; portable_empty_path = 'passed'; chocolatey_pack = 'passed';
        signed_system_executable = $SignedPath; unsigned_cli_rejected = $true;
        chocolatey = $ChocolateyPath; signtool = $SignToolPath
        packaged_install_upgrade_two_rollbacks = $(if ($PSCmdlet.ParameterSetName -eq 'Archive') { 'passed' } else { 'not requested' })
    } | ConvertTo-Json
} finally {
    foreach ($Name in $SavedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($Name, $SavedEnvironment[$Name], 'Process')
    }
    if ($KeepArtifacts) { Write-Host "Fixture retained: $FixtureRoot" }
    else { Remove-Item -LiteralPath $FixtureRoot -Recurse -Force }
}
