param(
    [Parameter(Mandatory)][string]$OutputPath,
    [ValidateSet('missing','stale')][string]$Fault = 'missing',
    [ValidateRange(30,120)][int]$Seconds = 100,
    [ValidateRange(10,119)][int]$FaultAfterSeconds = 80,
    [ValidateRange(0,30)][int]$KillAfterSeconds = 0
)
$ErrorActionPreference = 'Stop'
if ($FaultAfterSeconds -ge $Seconds) { throw 'Fault time must precede the controller deadline.' }
if ($KillAfterSeconds -gt 0 -and $KillAfterSeconds -ge $FaultAfterSeconds) { throw 'Crash test must precede the injected sensor fault.' }
$projectRoot = Split-Path -Parent $PSScriptRoot
$reportPath = [IO.Path]::GetFullPath((Join-Path $projectRoot $OutputPath))
$recoveryPath = [IO.Path]::ChangeExtension($reportPath, 'recovery.json')
$guardianPath = [IO.Path]::ChangeExtension($reportPath, 'guardian.json')
$stdoutPath = [IO.Path]::ChangeExtension($reportPath, 'stdout.log')
$stderrPath = [IO.Path]::ChangeExtension($reportPath, 'stderr.log')
$restoreOutPath = [IO.Path]::ChangeExtension($reportPath, 'restore.stdout.log')
$restoreErrPath = [IO.Path]::ChangeExtension($reportPath, 'restore.stderr.log')
foreach ($path in @($reportPath,$recoveryPath,$guardianPath,$stdoutPath,$stderrPath,$restoreOutPath,$restoreErrPath)) {
    if (Test-Path -LiteralPath $path) { throw "Refusing to overwrite $path" }
}
$controllerExe = Join-Path $projectRoot 'target\debug\rhelper-curve-test.exe'
$recoveryExe = Join-Path $projectRoot 'target\debug\rhelper-diagnostics.exe'
foreach ($path in @($controllerExe,$recoveryExe)) { if (!(Test-Path -LiteralPath $path)) { throw "Build the diagnostic first: $path" } }
# Windows paths cannot contain a double quote; quote the path as one argv item.
if ($reportPath.Contains('"')) { throw 'Invalid report path.' }
$argsList = @('--output',('"'+$reportPath+'"'),'--seconds',$Seconds,'--fault-after-seconds',$FaultAfterSeconds,'--fault',$Fault)
$child = $null
$guardian = [ordered]@{started_utc=[DateTime]::UtcNow.ToString('o');controller_pid=$null;exit_code=$null;killed=$false;reason=$null;recovery_verified=$false;recovered_state=$null;error=$null}
$failure = $null
try {
    $child = Start-Process -FilePath $controllerExe -ArgumentList $argsList -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $guardian.controller_pid = $child.Id
    $timer = [Diagnostics.Stopwatch]::StartNew()
    while (!$child.WaitForExit(1000)) {
        if (($KillAfterSeconds -gt 0 -and $timer.Elapsed.TotalSeconds -ge $KillAfterSeconds) -or $timer.Elapsed.TotalSeconds -ge ($Seconds+20)) {
            $guardian.reason = if ($KillAfterSeconds -gt 0) {'deliberate controller termination'} else {'outer deadline exceeded'}
            $child.Kill()
            $child.WaitForExit()
            $guardian.killed = $true
            break
        }
    }
    $child.WaitForExit()
    $guardian.exit_code = $child.ExitCode
    if (!$guardian.killed -and $child.ExitCode -ne 0) { throw "Controller failed with exit code $($child.ExitCode); inspect its report/log." }
    if ($guardian.killed -and $KillAfterSeconds -eq 0) { throw 'Controller exceeded its outer deadline.' }
} catch { $failure = $_.Exception.Message }
finally {
    if ($null -ne $child -and !$child.HasExited) { $child.Kill(); $child.WaitForExit() }
    if (Test-Path -LiteralPath $recoveryPath) {
        $restorer = $null
        try {
            $restorer = Start-Process -FilePath $recoveryExe -ArgumentList @('--restore-state',('"'+$recoveryPath+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput $restoreOutPath -RedirectStandardError $restoreErrPath
            if (!$restorer.WaitForExit(60000)) { $restorer.Kill(); $restorer.WaitForExit(); throw 'Recovery process exceeded its deadline.' }
            $restorer.WaitForExit()
            if ($restorer.ExitCode -ne 0) { throw "Recovery failed; inspect $restoreErrPath" }
            $guardian.recovered_state = Get-Content -LiteralPath $restoreOutPath -Raw | ConvertFrom-Json
            $guardian.recovery_verified = $true
        } catch { $failure = "$failure Recovery: $($_.Exception.Message)" }
        finally { if ($null -ne $restorer) { $restorer.Dispose() } }
    } else { $failure = "$failure No recovery snapshot was created; controller must not have written fan targets." }
    $guardian.error = $failure
    $guardian.finished_utc = [DateTime]::UtcNow.ToString('o')
    $guardian | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $guardianPath -Encoding utf8
    if ($null -ne $child) { $child.Dispose() }
}
Write-Output "killed=$($guardian.killed) recovery_verified=$($guardian.recovery_verified) report=$guardianPath"
if ($failure) { throw $failure }
