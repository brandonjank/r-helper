[CmdletBinding()]
param([Parameter(Mandatory)][string]$ArchivePath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [IO.Path]::GetFullPath($ArchivePath)
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    $manifestEntry = $zip.GetEntry('manifest.json')
    if (-not $manifestEntry) { throw 'Manifest missing from package.' }
    $reader = [IO.StreamReader]::new($manifestEntry.Open())
    try { $manifest = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    foreach ($required in @($manifest.binaries) + @('rhelper-thermal.exe', 'rhelper-controls.exe', 'LICENSE', 'NOTICE', 'THIRD_PARTY_LICENSES.md', 'portable.md', 'blade18-validation-batch.md')) {
        if (-not $zip.GetEntry($required)) { throw "Missing package entry: $required" }
    }
    if ($zip.GetEntry('rhelper-gpu-test.exe') -or $zip.GetEntry('rhelper-curve-test.exe')) { throw 'Developer workload included in portable package.' }
    $checksumEntry = $zip.GetEntry('SHA256SUMS.txt')
    if (-not $checksumEntry) { throw 'Checksums missing.' }
    $reader = [IO.StreamReader]::new($checksumEntry.Open())
    try { $lines = $reader.ReadToEnd() -split '\r?\n' | Where-Object { $_ } } finally { $reader.Dispose() }
    $covered = @{}
    foreach ($line in $lines) {
        if ($line -notmatch '^([0-9a-f]{64})  (.+)$') { throw "Invalid checksum line: $line" }
        $expected = $Matches[1]
        $name = $Matches[2]
        if ($covered.ContainsKey($name)) { throw "Duplicate checksum: $name" }
        $entry = $zip.GetEntry($name)
        if (-not $entry) { throw "Checksum names missing file: $name" }
        $stream = $entry.Open()
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $actual = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $stream.Dispose(); $sha.Dispose() }
        if ($actual -ne $expected) { throw "Checksum mismatch: $name" }
        $covered[$name] = $true
    }
    foreach ($entry in $zip.Entries) {
        if ($entry.FullName -ne 'SHA256SUMS.txt' -and -not $covered.ContainsKey($entry.FullName)) { throw "Uncovered package entry: $($entry.FullName)" }
    }
} finally { $zip.Dispose() }

$before = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
$rejected = $false
try { & (Join-Path $PSScriptRoot 'Package-Portable.ps1') -OutputDirectory $archive.Substring(0, $archive.Length - 4) | Out-Null }
catch { if ($_.Exception.Message -notlike '*destination already exists*') { throw }; $rejected = $true }
if (-not $rejected -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $before) { throw 'Overwrite refusal did not preserve the package.' }
$missing = Join-Path (Split-Path $PSScriptRoot -Parent) "target/package-missing-$([Guid]::NewGuid().ToString('N'))"
$rejected = $false
try { & (Join-Path $PSScriptRoot 'Package-Portable.ps1') -BinaryDirectory "$missing-input" -OutputDirectory $missing | Out-Null }
catch { if ($_.Exception.Message -notlike '*Required package file is missing*') { throw }; $rejected = $true }
if (-not $rejected -or (Test-Path -LiteralPath $missing)) { throw 'Incomplete package was not rejected before output creation.' }
Write-Output 'Portable package contents, checksums, overwrite refusal and missing-input checks passed.'
