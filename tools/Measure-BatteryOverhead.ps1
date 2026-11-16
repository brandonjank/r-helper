param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateSet('Closed','Hidden','Visible')][string]$Mode='Hidden',
    [ValidateRange(30,1200)][int]$Seconds=300,
    [switch]$AllowAC
)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$directory=if([IO.Path]::IsPathRooted($OutputDirectory)){[IO.Path]::GetFullPath($OutputDirectory)}else{[IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))}
if(Test-Path -LiteralPath $directory){throw 'Choose a new output directory.'}
if(Get-Process rhelper -ErrorAction SilentlyContinue){throw 'Exit R-Helper before this controlled sample.'}
[IO.Directory]::CreateDirectory($directory)|Out-Null
$exe=Join-Path $root 'target\release\rhelper.exe'
$settingsFile=Join-Path $env:LOCALAPPDATA 'RHelper\blade18.json'
if(Test-Path $settingsFile){$settings=Get-Content $settingsFile -Raw|ConvertFrom-Json;if($settings.application_switching -or $settings.lighting_policy.enabled){throw 'Application switching or lighting policies would confound this idle comparison; disable them for the controlled batch.'}}
if(!('RHelperMeasurementPower' -as [type])){Add-Type 'using System.Runtime.InteropServices; public static class RHelperMeasurementPower { [DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint flags); }'}
$probe=Join-Path $root 'diagnostic-reports\research\BatteryProbe.exe'
if(!(Test-Path $probe)){
    & "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe" /nologo /platform:x64 /r:System.Web.Extensions.dll "/out:$probe" (Join-Path $PSScriptRoot 'BatteryProbe.cs')
    if($LASTEXITCODE -ne 0){throw 'Battery probe compilation failed.'}
}
$probeOutput=Join-Path $directory 'battery-preflight.json'
& $probe --output $probeOutput | Out-Null
if($LASTEXITCODE -ne 0){throw 'Battery-class preflight failed.'}
$battery=(Get-Content $probeOutput -Raw|ConvertFrom-Json).batteries
if(@($battery).Count -ne 1 -or $battery[0].information.capacity_is_relative -ne $false){throw 'Absolute battery capacity units are unavailable.'}
$full=[double]$battery[0].information.full_charge_capacity_raw
if($full -le 0 -or $full -gt 500000){throw 'Full-charge capacity is unknown or outside the laptop measurement envelope.'}
function Read-Battery {
    $b=@(Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus)
    if($b.Count -ne 1){throw 'Expected one battery.'}
    if($b[0].RemainingCapacity -gt $full*1.15){throw 'Remaining capacity is unavailable or inconsistent with the battery-class preflight.'}
    if($AllowAC){if(!$b[0].PowerOnline){throw 'AC dry-run power state changed.'}}
    elseif($b[0].PowerOnline -or !$b[0].Discharging){throw 'Battery test requires AC unplugged and active discharge.'}
    if($b[0].RemainingCapacity -lt $full*0.25){throw 'Battery below 25%; reconnect AC.'}
    [pscustomobject]@{utc=[DateTime]::UtcNow.ToString('o');remaining_mwh=[double]$b[0].RemainingCapacity;discharge_mw=$(if($b[0].DischargeRate -eq [uint32]::MaxValue){$null}else{$b[0].DischargeRate});ac=[bool]$b[0].PowerOnline;discharging=[bool]$b[0].Discharging}
}
function Snapshot([string]$name) {
    $path=Join-Path $directory "$name-display.json"
    & (Join-Path $root 'target\release\rhelper-display.exe') --output $path | Out-Null
    if($LASTEXITCODE -ne 0){throw 'Display inventory unavailable.'}
    $display=Get-Content $path -Raw|ConvertFrom-Json
    $brightness=@(Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness -ErrorAction SilentlyContinue|Select-Object -ExpandProperty CurrentBrightness)
    [pscustomobject]@{display=$display;brightness=$brightness;power_scheme=((& powercfg.exe /getactivescheme) -join '');settings_hash=$(if(Test-Path $settingsFile){(Get-FileHash $settingsFile -Algorithm SHA256).Hash}else{$null})}
}
$app=$null;$failure=$null;$samples=[Collections.Generic.List[object]]::new();$result=$null;$previousExecution=$null
try {
    $previousExecution=[RHelperMeasurementPower]::SetThreadExecutionState(2147483651)
    if($previousExecution -eq 0){throw 'Could not hold display/system awake for the controlled sample.'}
    $preflight=Read-Battery
    $before=Snapshot 'before'
    $processReport=Join-Path $directory 'process.json'
    if($Mode -ne 'Closed'){
        $app=Start-Process -FilePath $exe -ArgumentList @('--idle-benchmark',$Mode.ToLowerInvariant(),$Seconds,('"'+$processReport+'"')) -PassThru -WindowStyle Hidden
    }
    Start-Sleep -Seconds 15
    $timer=[Diagnostics.Stopwatch]::StartNew()
    do {
        $runningApps=@(Get-Process rhelper -ErrorAction SilentlyContinue)
        if($Mode -eq 'Closed' -and $runningApps.Count -ne 0){throw 'R-Helper started during the closed baseline.'}
        if($Mode -ne 'Closed' -and @($runningApps|Where-Object Id -ne $app.Id).Count -ne 0){throw 'Another R-Helper process started during sampling.'}
        if($null -ne $app -and $app.HasExited){
            if(!(Test-Path $processReport) -or (Get-Item $processReport).Length -eq 0){throw 'R-Helper exited before writing its measurement.'}
            $ended=Get-Content $processReport -Raw|ConvertFrom-Json
            if($ended.error -or $ended.elapsed_seconds -lt $Seconds-1){throw 'R-Helper did not complete its measurement.'}
        }
        $sample=Read-Battery;$sample|Add-Member -NotePropertyName elapsed_seconds -NotePropertyValue $timer.Elapsed.TotalSeconds
        if($samples.Count -gt 0 -and $sample.elapsed_seconds-$samples[$samples.Count-1].elapsed_seconds -gt 30){throw 'Sampling gap or sleep detected; this phase is not a controlled idle sample.'}
        $samples.Add($sample)
        $remaining=$Seconds-$timer.Elapsed.TotalSeconds
        if($remaining -le 0){break}
        Start-Sleep -Milliseconds ([int]([Math]::Min(15,$remaining)*1000))
    }while($true)
    if($null -ne $app){if(!$app.WaitForExit(15000)){throw 'Finite idle benchmark did not exit.'};if(!(Test-Path $processReport) -or (Get-Item $processReport).Length -eq 0){throw 'Missing process report.'};$processData=Get-Content $processReport -Raw|ConvertFrom-Json;if($processData.error -or $processData.elapsed_seconds -lt $Seconds-1 -or $processData.mode -ne $Mode.ToLowerInvariant()){throw 'Process measurement was incomplete or failed.'}}
    $after=Snapshot 'after'
    $elapsed=$samples[$samples.Count-1].elapsed_seconds-$samples[0].elapsed_seconds
    $energy=$samples[0].remaining_mwh-$samples[$samples.Count-1].remaining_mwh
    $matched=($before|ConvertTo-Json -Depth 15 -Compress) -eq ($after|ConvertTo-Json -Depth 15 -Compress)
    $watts=$(if($elapsed -gt 0){$energy*3.6/$elapsed}else{$null})
    $valid=!$AllowAC -and $matched -and $watts -gt 0 -and $watts -le 400 -and $elapsed -ge 60
    $result=[ordered]@{mode=$Mode;ac_dry_run=[bool]$AllowAC;elapsed_seconds=$elapsed;energy_mwh=$energy;average_discharge_watts=$(if($valid){$watts}else{$null});battery_energy_valid=$valid;matched_display_brightness_power_scheme=$matched;before=$before;after=$after;samples=$samples;process=$(if(Test-Path $processReport){Get-Content $processReport -Raw|ConvertFrom-Json}else{$null});note='No NVIDIA queries during sampling. GPU residency, lighting and background workload are not independently proven constant; battery results need repeated matched trials. Vendor takeover skipped in finite GUI benchmark.'}
}catch{$failure=$_.Exception.Message}
finally {
    if($null -ne $previousExecution -and $previousExecution -ne 0){[RHelperMeasurementPower]::SetThreadExecutionState($previousExecution)|Out-Null}
    if($null -ne $app){if(!$app.HasExited){$app.Refresh();if([IO.Path]::GetFullPath($app.Path) -eq [IO.Path]::GetFullPath($exe)){$app.Kill();$app.WaitForExit()}};$app.Dispose()}
    if($null -eq $result){$result=[ordered]@{mode=$Mode;battery_energy_valid=$false;samples=$samples}}
    $result.error=$failure
    $result|ConvertTo-Json -Depth 20|Set-Content (Join-Path $directory 'result.json') -Encoding utf8
}
if($failure){throw $failure}
$result|Select-Object mode,ac_dry_run,elapsed_seconds,energy_mwh,average_discharge_watts,battery_energy_valid,matched_display_brightness_power_scheme,process,error|ConvertTo-Json -Depth 4
