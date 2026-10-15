param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateSet('Stop','Timeout','WorkerExit','WatchdogExit','ParentExit','AcLoss','SleepResume')][string]$Scenario='Stop',
    [ValidateSet('debug','release')][string]$BuildConfiguration='release'
)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$directory=if([IO.Path]::IsPathRooted($OutputDirectory)){[IO.Path]::GetFullPath($OutputDirectory)}else{[IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))}
if(Test-Path -LiteralPath $directory){throw 'Choose a new session directory.'}
[IO.Directory]::CreateDirectory($directory)|Out-Null
$exe=Join-Path $root "target\$BuildConfiguration\rhelper.exe"
$thermalExe=Join-Path $root "target\$BuildConfiguration\rhelper-thermal.exe"
$config=@{schema=1;downward_hold_seconds=5;session_seconds=$(if($Scenario -eq 'Timeout'){30}else{120});zones=@(
    @{source='EcA';points=@(@{celsius=50;rpm=4000},@{celsius=80;rpm=4500})},
    @{source='EcB';points=@(@{celsius=35;rpm=4000},@{celsius=70;rpm=4500})},
    @{source='Hottest';points=@(@{celsius=50;rpm=3000},@{celsius=80;rpm=3500})}
)}
$config|ConvertTo-Json -Depth 6|Set-Content (Join-Path $directory 'config.json') -Encoding utf8NoBOM
function Read-Thermal([string]$label){
    for($attempt=1;$attempt -le 3;$attempt++){
        $path=Join-Path $directory "$label-$attempt.jsonl";$log=Join-Path $directory "$label-$attempt.stderr.log"
        & $thermalExe --seconds 1 --output $path 2>$log
        if($LASTEXITCODE -eq 0){return (Get-Content $path|Select-Object -Last 1|ConvertFrom-Json)}
        Start-Sleep -Milliseconds 200
    }
    throw "Independent thermal read failed; inspect $label-*.stderr.log"
}
$before=Read-Thermal 'before'
$supervisor=$null;$parent=$null;$worker=$null;$failure=$null;$test=[ordered]@{scenario=$Scenario;passed=$false;before=$before;active=$null;finished=$null;after=$null;error=$null}
try {
    $parentId=$PID
    if($Scenario -eq 'ParentExit'){
        $parent=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 300') -PassThru -WindowStyle Hidden
        $parentId=$parent.Id
    }
    $supervisor=Start-Process -FilePath $exe -ArgumentList @('--curve-supervisor',('"'+$directory+'"'),$parentId) -PassThru -WindowStyle Hidden
    $deadline=[DateTime]::UtcNow.AddSeconds(25)
    do {
        if(Test-Path (Join-Path $directory 'finished.json')){throw 'Session stopped before becoming active.'}
        $status=$null
        try {$status=Get-Content (Join-Path $directory 'status.json') -Raw -ErrorAction Stop|ConvertFrom-Json}catch{}
        if($status.phase -eq 'running' -and $status.elapsed_seconds -ge 10){break}
        Start-Sleep -Milliseconds 200
    }while([DateTime]::UtcNow -lt $deadline)
    if($status.phase -ne 'running'){throw 'Controller did not become active.'}
    $test.active=$status
    switch($Scenario){
        'Stop' {Set-Content (Join-Path $directory 'stop') 'stop'}
        'WorkerExit' {
            $ready=Get-Content (Join-Path $directory 'ready.json') -Raw|ConvertFrom-Json
            $worker=Get-Process -Id $ready.worker_pid
            if([IO.Path]::GetFullPath($worker.Path) -ne [IO.Path]::GetFullPath($exe)){throw 'Worker executable identity mismatch.'}
            $worker.Kill();$worker.WaitForExit()
        }
        'WatchdogExit' {$supervisor.Kill();$supervisor.WaitForExit()}
        'ParentExit' {$parent.Kill();$parent.WaitForExit()}
        'AcLoss' {@{scenario=$Scenario;instruction='Unplug AC now and leave it unplugged until restoration is verified.'}|ConvertTo-Json|Set-Content (Join-Path $directory 'awaiting-owner.json')}
        'SleepResume' {@{scenario=$Scenario;instruction='Sleep for at least 15 seconds, then wake and unlock. Owner confirmation is required to establish physical sleep.'}|ConvertTo-Json|Set-Content (Join-Path $directory 'awaiting-owner.json')}
    }
    $deadline=[DateTime]::UtcNow.AddSeconds($(if($Scenario -in @('AcLoss','SleepResume')){120}else{45}))
    do {
        if($Scenario -eq 'WatchdogExit'){
            $status=Get-Content (Join-Path $directory 'status.json') -Raw|ConvertFrom-Json
            if($status.phase -eq 'stopped'){
                if(@($status.sample.modes|Where-Object {$_[1] -ne 0}).Count -gt 0){throw 'Worker did not restore Auto after watchdog loss.'}
                $test.finished=$status;break
            }
        }elseif(Test-Path (Join-Path $directory 'finished.json')){
            $finished=Get-Content (Join-Path $directory 'finished.json') -Raw|ConvertFrom-Json
            if(!$finished.recovery_verified){throw 'Watchdog recovery was not verified.'}
            if($Scenario -eq 'AcLoss'){
                if($finished.status.error -notmatch 'AC disconnected or unknown'){throw 'Recovered for another reason; AC-loss fallback is not verified.'}
                $battery=@(Get-CimInstance -Namespace root/wmi -ClassName BatteryStatus)
                if($battery.Count -ne 1 -or $battery[0].PowerOnline){throw 'AC disconnection was not independently observed.'}
            }
            if($Scenario -eq 'SleepResume'){
                $reason=(@($finished.status.error,$finished.reason) -join ' ')
                if($reason -notmatch 'Resume|heartbeat'){throw 'Recovered for another reason; delayed-worker/resume fallback is not verified.'}
                $test['requires_owner_sleep_confirmation']=$true
            }
            $test.finished=$finished;break
        }
        Start-Sleep -Milliseconds 200
    }while([DateTime]::UtcNow -lt $deadline)
    if($null -eq $test.finished){throw 'Expected automatic recovery did not finish in time.'}
    $after=Read-Thermal 'after'
    $test.after=$after
    if(@($after.sample.modes|Where-Object {$_[1] -ne 0}).Count -gt 0 -or ($after.sample.target_rpm -join ',') -ne ($before.sample.target_rpm -join ',')){throw 'Final Auto modes or original targets differ.'}
    $test.passed=$true
}catch{$failure=$_.Exception.Message}
finally {
    Set-Content (Join-Path $directory 'stop') 'stop'
    if($null -ne $supervisor -and !$supervisor.HasExited){if(!$supervisor.WaitForExit(10000)){$supervisor.Kill();$supervisor.WaitForExit()}}
    if(Test-Path (Join-Path $directory 'recovery.json')){
        $restorer=Start-Process -FilePath $exe -ArgumentList @('--curve-recover',('"'+$directory+'"')) -PassThru -WindowStyle Hidden
        if(!$restorer.WaitForExit(30000)){$restorer.Kill();$restorer.WaitForExit();$failure="$failure Final defensive recovery timed out."}else{$restorer.WaitForExit();if($restorer.ExitCode -ne 0){$failure="$failure Final defensive recovery failed."}}
        $restorer.Dispose()
    }
    if($null -ne $parent){if(!$parent.HasExited){$parent.Kill();$parent.WaitForExit()};$parent.Dispose()}
    if($null -ne $worker){$worker.Dispose()}
    if($null -ne $supervisor){$supervisor.Dispose()}
    $test.error=$failure
    $test|ConvertTo-Json -Depth 10|Set-Content (Join-Path $directory 'test-result.json') -Encoding utf8
}
Write-Output "$Scenario passed=$($test.passed) directory=$directory"
if($failure){throw $failure}
