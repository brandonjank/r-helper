param([Parameter(Mandatory)][string]$OutputDirectory,[ValidateRange(120,1200)][int]$SecondsPerPhase=300)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$directory=if([IO.Path]::IsPathRooted($OutputDirectory)){[IO.Path]::GetFullPath($OutputDirectory)}else{[IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))}
if(Test-Path -LiteralPath $directory){throw 'Choose a new batch directory.'}
[IO.Directory]::CreateDirectory($directory)|Out-Null
$results=@();$index=0
if(!('RHelperMeasurementPower' -as [type])){Add-Type 'using System.Runtime.InteropServices; public static class RHelperMeasurementPower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); }'}
$previousExecution=[RHelperMeasurementPower]::SetThreadExecutionState(2147483651)
if($previousExecution -eq 0){throw 'Could not hold display/system awake for ABBA comparison.'}
try {
foreach($mode in @('Closed','Hidden','Hidden','Closed')){
    $phase=Join-Path $directory ("phase-$index-$mode")
    & (Join-Path $PSScriptRoot 'Measure-BatteryOverhead.ps1') -OutputDirectory $phase -Mode $mode -Seconds $SecondsPerPhase | Out-Null
    $report=Get-Content (Join-Path $phase 'result.json') -Raw|ConvertFrom-Json
    if(!$report.battery_energy_valid){throw 'A phase did not produce a valid battery-energy measurement; inspect preserved results.'}
    $results+=,$report;$index++
}
}finally{[RHelperMeasurementPower]::SetThreadExecutionState($previousExecution)|Out-Null}
$closed=@($results|Where-Object mode -eq 'Closed'|Select-Object -ExpandProperty average_discharge_watts)
$hidden=@($results|Where-Object mode -eq 'Hidden'|Select-Object -ExpandProperty average_discharge_watts)
$allMatched=@($results|Where-Object {($_.before|ConvertTo-Json -Depth 15 -Compress) -ne ($results[0].before|ConvertTo-Json -Depth 15 -Compress)}).Count -eq 0
$summary=[ordered]@{order=@('Closed','Hidden','Hidden','Closed');closed_watts=$closed;hidden_watts=$hidden;all_phase_controls_match=$allMatched;estimated_added_watts=$(if($allMatched){($hidden|Measure-Object -Average).Average-($closed|Measure-Object -Average).Average}else{$null});note='One ABBA batch is an estimate, not proof of causal app overhead. Compare repeatability and background workload; no GPU polling or vendor takeover during measurement.'}
$summary|ConvertTo-Json -Depth 5|Set-Content (Join-Path $directory 'summary.json') -Encoding utf8
$summary|ConvertTo-Json -Depth 5
