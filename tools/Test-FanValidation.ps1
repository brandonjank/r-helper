param([Parameter(Mandatory)][string]$OutputDirectory,[switch]$Guard,[int]$OwnerId=0)
$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$directory=if([IO.Path]::IsPathRooted($OutputDirectory)){[IO.Path]::GetFullPath($OutputDirectory)}else{[IO.Path]::GetFullPath((Join-Path $root $OutputDirectory))}
$exe=Join-Path $root 'target\release\rhelper-diagnostics.exe'
$thermalExe=Join-Path $root 'target\release\rhelper-thermal.exe'
$recovery=Join-Path $directory 'original.recovery.json'
if($Guard){
    $owner=Get-Process -Id $OwnerId
    $ownerHandle=$owner.SafeHandle
    'ready'|Set-Content (Join-Path $directory 'guard.ready')
    $deadline=[DateTime]::UtcNow.AddSeconds(240)
    while(!$owner.HasExited -and [DateTime]::UtcNow -lt $deadline -and !(Test-Path (Join-Path $directory 'finished')) -and !(Test-Path (Join-Path $directory 'recover-now'))){Start-Sleep -Milliseconds 200;$owner.Refresh()}
    $fallback=!(Test-Path (Join-Path $directory 'finished'))
    $restored=$true;$failure=$null
    if($fallback){
        try {
            $workerFile=Join-Path $directory 'worker.json'
            if(Test-Path $workerFile){
                $record=Get-Content $workerFile -Raw|ConvertFrom-Json
                $worker=Get-Process -Id $record.process_id -ErrorAction SilentlyContinue
                if($null -ne $worker){
                    if([IO.Path]::GetFullPath($worker.Path) -ne [IO.Path]::GetFullPath($exe) -or $worker.StartTime.ToUniversalTime().Ticks -ne $record.started_ticks){throw 'Worker identity mismatch; refusing to terminate another process.'}
                    $worker.Kill();$worker.WaitForExit();$worker.Dispose()
                }
            }
            & $exe --restore-state $recovery > (Join-Path $directory 'guard-restore.log') 2>&1
            if($LASTEXITCODE -ne 0){throw 'Independent fan recovery failed.'}
        }catch{$restored=$false;$failure=$_.Exception.Message}
    }
    @{fallback=$fallback;restored=$restored;error=$failure}|ConvertTo-Json|Set-Content (Join-Path $directory 'guard-result.json')
    $owner.Dispose()
    if(!$restored){throw $failure};return
}
if(Test-Path -LiteralPath $directory){throw 'Choose a new output directory.'}
if(Get-Process rhelper -ErrorAction SilentlyContinue){throw 'Exit R-Helper before the isolated fan validation.'}
[IO.Directory]::CreateDirectory($directory)|Out-Null
& $exe --capture-state --output (Join-Path $directory 'original-state.json') | Out-Null
if($LASTEXITCODE -ne 0){throw 'Could not capture original hardware state.'}
& $thermalExe --seconds 1 --output (Join-Path $directory 'original-thermal.jsonl') 2> (Join-Path $directory 'original-thermal.log')
if($LASTEXITCODE -ne 0){throw 'Could not capture original thermal state.'}
$state=Get-Content (Join-Path $directory 'original-state.json') -Raw|ConvertFrom-Json
$thermal=(Get-Content (Join-Path $directory 'original-thermal.jsonl')|Select-Object -Last 1|ConvertFrom-Json).sample
if(@($thermal.modes|Where-Object {$_[1] -ne 0}).Count -ne 0 -or $thermal.ec_celsius[0] -le 0 -or $thermal.ec_celsius[0] -ge 80 -or $thermal.ec_celsius[1] -le 0 -or $thermal.ec_celsius[1] -ge 70){throw 'Automatic cooling and initial temperature headroom required.'}
@{model='RZ09-0529';original=$state.original;thermal_original=$thermal}|ConvertTo-Json -Depth 8|Set-Content $recovery -Encoding utf8NoBOM
$guardProcess=$null;$fanProcess=$null;$failure=$null;$restored=$false
try {
    $guardProcess=Start-Process -FilePath (Join-Path $PSHOME 'pwsh.exe') -ArgumentList @('-NoProfile','-File',('"'+$PSCommandPath+'"'),'-Guard','-OwnerId',$PID,'-OutputDirectory',('"'+$directory+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $directory 'guard.log') -RedirectStandardError (Join-Path $directory 'guard.stderr.log')
    $deadline=[DateTime]::UtcNow.AddSeconds(5)
    while(!(Test-Path (Join-Path $directory 'guard.ready'))){if($guardProcess.HasExited -or [DateTime]::UtcNow -ge $deadline){throw 'Independent guard not ready; no fan writes sent.'};Start-Sleep -Milliseconds 100}
    $fanProcess=Start-Process -FilePath $exe -ArgumentList @('--validate-fan-zones','--output',('"'+(Join-Path $directory 'fan-zones.json')+'"')) -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $directory 'validation.log') -RedirectStandardError (Join-Path $directory 'validation.stderr.log')
    @{process_id=$fanProcess.Id;started_ticks=$fanProcess.StartTime.ToUniversalTime().Ticks}|ConvertTo-Json|Set-Content (Join-Path $directory 'worker.json')
    if(!$fanProcess.WaitForExit(220000)){throw 'Fan validation deadline reached.'}
    if($fanProcess.ExitCode -ne 0){throw 'Fan validation failed; inspect preserved readback logs.'}
}catch{$failure=$_.Exception.Message}
finally {
    if($null -ne $fanProcess){if(!$fanProcess.HasExited){$fanProcess.Kill();$fanProcess.WaitForExit()};$fanProcess.Dispose()}
    if($null -ne $fanProcess){
        & $exe --restore-state $recovery > (Join-Path $directory 'parent-restore.log') 2>&1
        $restored=$LASTEXITCODE -eq 0
    }else{$restored=$true}
    $parentRestored=$restored
    if($restored){'restored'|Set-Content (Join-Path $directory 'finished')}else{'retry'|Set-Content (Join-Path $directory 'recover-now')}
    if($null -ne $guardProcess){$guardProcess.WaitForExit(15000)|Out-Null;$guardProcess.Dispose()}
    if(!$restored -and (Test-Path (Join-Path $directory 'guard-result.json'))){$guardResult=Get-Content (Join-Path $directory 'guard-result.json') -Raw|ConvertFrom-Json;$restored=$guardResult.restored -eq $true}
    @{passed=($null -eq $failure -and $restored);parent_restored=$parentRestored;original_state_restored=$restored;error=$failure;physical_mapping_verified=$false;note='Vendor services were left in their original state; conflicting changes abort validation.'}|ConvertTo-Json|Set-Content (Join-Path $directory 'result.json')
}
if(!$restored){throw 'Original settings still need recovery; inspect guard-result.json.'}
if($failure){throw $failure}
Get-Content (Join-Path $directory 'result.json')
