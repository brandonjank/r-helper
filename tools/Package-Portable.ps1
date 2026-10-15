[CmdletBinding()]
param(
    [string]$BinaryDirectory,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
if (-not $BinaryDirectory) { $BinaryDirectory = Join-Path $repo 'target/release' }
$versionMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $repo 'Cargo.toml') -Raw), '(?m)^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"')
if (-not $versionMatch.Success) { throw 'Cannot read application version.' }
$version = $versionMatch.Groups[1].Value
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repo "dist/rhelper-$version-windows-x64-$(Get-Date -Format 'yyyyMMdd-HHmmss')" }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$archive = "$OutputDirectory.zip"
if ((Test-Path -LiteralPath $OutputDirectory) -or (Test-Path -LiteralPath $archive)) { throw 'Package destination already exists; choose a new destination.' }

# This list includes the thermal helper required for supervised curves, but no
# developer GPU workload or curve-test executables.
$binaries = @('rhelper.exe', 'rhelper-controls.exe', 'rhelper-diagnostics.exe', 'rhelper-display.exe', 'rhelper-features.exe', 'rhelper-thermal.exe')
$documents = @('LICENSE', 'NOTICE', 'THIRD_PARTY_LICENSES.md', 'docs/portable.md', 'docs/blade18-validation-batch.md')
$sources = @($binaries | ForEach-Object { Join-Path $BinaryDirectory $_ }) + @($documents | ForEach-Object { Join-Path $repo $_ })
foreach ($source in $sources) {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw "Required package file is missing: $source" }
}
$revision = & git -C $repo rev-parse HEAD
if ($LASTEXITCODE -ne 0) { throw 'Cannot identify source revision.' }
$dirty = @(& git -C $repo status --porcelain).Count -gt 0
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect source status.' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
foreach ($source in $sources) { Copy-Item -LiteralPath $source -Destination $OutputDirectory }
$manifest = [ordered]@{
    application = 'R-Helper'
    version = $version
    platform = 'windows-x64'
    sourceRevision = $revision.Trim()
    sourceHasUncommittedChanges = $dirty
    packagedAtUtc = [DateTime]::UtcNow.ToString('o')
    binaries = $binaries
    support = 'Existing upstream catalog and experimental Blade 18 (2025) RZ09-0529 support'
}
$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'manifest.json') -Encoding UTF8
$checksums = foreach ($file in Get-ChildItem -LiteralPath $OutputDirectory -File | Sort-Object Name) {
    "{0}  {1}" -f (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $file.Name
}
$checksums | Set-Content -LiteralPath (Join-Path $OutputDirectory 'SHA256SUMS.txt') -Encoding ASCII
Compress-Archive -Path (Join-Path $OutputDirectory '*') -DestinationPath $archive
Write-Output $archive
