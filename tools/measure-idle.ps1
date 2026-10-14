param(
    [int]$AppProcessId = 0,
    [ValidateRange(5, 60)][int]$Seconds = 30,
    [string]$Label = 'R-Helper idle sample',
    [string]$OutputPath
)
$ErrorActionPreference = 'Stop'
$appProcesses = if ($AppProcessId) { @(Get-Process -Id $AppProcessId) } else { @(Get-Process rhelper) }
if ($appProcesses.Count -ne 1) { throw 'Specify -AppProcessId when more than one R-Helper process is running.' }
$appProcess = $appProcesses[0]
$cpuBefore = $appProcess.TotalProcessorTime.TotalSeconds
$timer = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds $Seconds
$appProcess.Refresh()
if ($appProcess.HasExited) { throw 'The measured process exited during sampling.' }
$cpuSeconds = $appProcess.TotalProcessorTime.TotalSeconds - $cpuBefore
$elapsed = $timer.Elapsed.TotalSeconds
$result = [pscustomobject]@{
    label = $Label
    processId = $appProcess.Id
    timestampUtc = [DateTime]::UtcNow.ToString('o')
    elapsedSeconds = $elapsed
    cpuSeconds = $cpuSeconds
    percentOfOneCore = 100 * $cpuSeconds / $elapsed
    percentOfAllProcessors = 100 * $cpuSeconds / $elapsed / [Environment]::ProcessorCount
    workingSetBytes = $appProcess.WorkingSet64
    note = 'Process CPU sample only; does not measure GPU activity or battery energy.'
}
$json = $result | ConvertTo-Json
if ($OutputPath) {
    $stream = [IO.File]::Open($OutputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes($json + [Environment]::NewLine)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally { $stream.Dispose() }
}
$json
