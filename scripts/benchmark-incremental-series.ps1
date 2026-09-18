#requires -Version 7.0
param(
    [Parameter(Mandatory)][string[]]$Roms,
    [string]$RustExe = (Join-Path $PSScriptRoot '../target/release/ndstool-rs.exe'),
    [string]$OriginalExe,
    [string]$OutputRoot = (Join-Path $PSScriptRoot '../compat-bench/incremental-series'),
    [int]$Runs = 3
)
$ErrorActionPreference = 'Stop'
# Reuse quiet process timing and direct payload verification, without running
# the old independent-edit experiment. Preserve parameters across dot-sourcing.
$seriesRoms=$Roms; $seriesRoot=$OutputRoot; $seriesRuns=$Runs
. "$PSScriptRoot/benchmark-incremental.ps1" -HelpersOnly -RustExe $RustExe
$Roms=$seriesRoms; $OutputRoot=$seriesRoot; $Runs=$seriesRuns
if ($Runs -lt 1) { throw 'Runs must be positive' }
if ($OriginalExe) { $OriginalExe=(Resolve-Path -LiteralPath $OriginalExe).Path }
if (Test-Path -LiteralPath $OutputRoot) { throw 'Choose a fresh output directory.' }
New-Item -ItemType Directory -Path $OutputRoot | Out-Null
$OutputRoot=(Resolve-Path -LiteralPath $OutputRoot).Path
$rows=[Collections.Generic.List[object]]::new()
$initializations=[Collections.Generic.List[object]]::new()
$romNumber=0
foreach ($romPath in $Roms) {
    $romPath=(Resolve-Path -LiteralPath $romPath).Path
    $romNumber++
    $work=Join-Path $OutputRoot "rom-$romNumber"
    $data=Join-Path $work 'data'
    New-Item -ItemType Directory -Path $data | Out-Null
    $null=Invoke-Native @('-x',$romPath,'-d',$data,'-9',"$work/arm9.bin",'-7',"$work/arm7.bin",
        '-t',"$work/banner.bin",'-o',"$work/logo.bin",'-y9',"$work/arm9ovr.bin",'-y7',"$work/arm7ovr.bin",
        '-y',"$work/overlays",'--parallel','--incremental')
    [IO.File]::WriteAllBytes("$work/header.bin",[NdsBench]::Read($romPath,0,512))
    $cache=Join-Path $data '.ndstool-rs'
    $state=[IO.File]::ReadAllBytes("$cache/layout.bin")
    $snapshot=@(Get-ChildItem -LiteralPath $cache -Filter 'snapshot-*.nds')[0].FullName
    [IO.File]::Copy($snapshot,"$work/baseline.nds")
    $files=@([NdsBench]::Index($romPath).GetEnumerator() | Where-Object { $_.Value.Size -ge 2 } | Sort-Object { $_.Value.Start })
    if ($files.Count -lt 10) { throw 'Need at least ten nonempty payloads.' }
    $middle=[math]::Min([int][math]::Floor($files.Count/2),$files.Count-10)
    $selected=@($files[$middle..($middle+9)])
    $originals=@{}
    foreach ($entry in $selected) {
        $path=Join-Path $data $entry.Key.TrimStart('/')
        $originals[$path]=@{Bytes=[IO.File]::ReadAllBytes($path); Time=[IO.File]::GetLastWriteTimeUtc($path)}
    }
    $added=@(1..10 | ForEach-Object { Join-Path (Split-Path -Parent (Join-Path $data $selected[0].Key.TrimStart('/'))) "__ndstool_series_$_.bin" })
    foreach ($path in $added) { if (Test-Path -LiteralPath $path) { throw 'Benchmark filename collision' } }
    $fullArgs=@('-9',"$work/arm9.bin",'-7',"$work/arm7.bin",'-t',"$work/banner.bin",'-o',"$work/logo.bin",
        '-h',"$work/header.bin",'-d',$data,'-y9',"$work/arm9ovr.bin",'-y7',"$work/arm7ovr.bin",'-y',"$work/overlays")
    foreach ($scenario in @('Added','Removed','Grown','Shrunk')) {
        for ($run=1; $run -le $Runs; $run++) {
            # Only reset between complete ten-step series. Every step reuses
            # the PREVIOUS successful build and the SAME independent output.
            foreach ($path in $originals.Keys) {
                [IO.File]::WriteAllBytes($path,$originals[$path].Bytes)
                [IO.File]::SetLastWriteTimeUtc($path,$originals[$path].Time)
            }
            foreach ($path in $added) { if ([IO.File]::Exists($path)) { [IO.File]::Delete($path) } }
            [IO.File]::Copy("$work/baseline.nds",$snapshot,$true)
            [IO.File]::WriteAllBytes("$cache/layout.bin",$state)
            if ([IO.File]::Exists("$cache/output.receipt")) { [IO.File]::Delete("$cache/output.receipt") }
            $incOut="$work/$scenario-incremental.nds"; $fullOut="$work/$scenario-full.nds"
            $originalOut="$work/$scenario-original.nds"
            $init=Invoke-Native @('-c',$incOut,'-d',$data,'--incremental')
            $initializations.Add([pscustomobject]@{Rom=[IO.Path]::GetFileNameWithoutExtension($romPath);Scenario=$scenario;Run=$run;Ms=$init.Ms})
            $null=Invoke-Native (@('-c',$fullOut)+$fullArgs)
            if ($OriginalExe) { $null=Invoke-Native (@('-c',$originalOut)+$fullArgs) -Executable $OriginalExe }
            $fullTotal=0.0; $incTotal=0.0; $originalTotal=0.0
            for ($step=1; $step -le 10; $step++) {
                $path=Join-Path $data $selected[$step-1].Key.TrimStart('/')
                $bytes=$originals[$path].Bytes
                switch ($scenario) {
                    'Added' { $b=[byte[]]::new(65536); $b[0]=[byte]$step; [IO.File]::WriteAllBytes($added[$step-1],$b) }
                    'Removed' { [IO.File]::Delete($path) }
                    'Grown' { $b=[byte[]]::new($bytes.Length+65536); [Array]::Copy($bytes,$b,$bytes.Length); $b[-1]=[byte]$step; [IO.File]::WriteAllBytes($path,$b) }
                    'Shrunk' { [IO.File]::WriteAllBytes($path,$bytes[0..([int][math]::Floor($bytes.Length/2)-1)]) }
                }
                $originalResult=$null
                if ($OriginalExe) {
                    # Rotate all three modes through first/middle/last positions.
                    $order=@('Original','Full','Incremental')
                    $rotation=($step+$run)%3
                    for ($position=0; $position -lt 3; $position++) {
                        switch ($order[($position+$rotation)%3]) {
                            'Original' { $originalResult=Invoke-Native (@('-c',$originalOut)+$fullArgs) -Executable $OriginalExe }
                            'Full' { $full=Invoke-Native (@('-c',$fullOut)+$fullArgs) }
                            'Incremental' { $inc=Invoke-Native @('-c',$incOut,'-d',$data,'--incremental') }
                        }
                    }
                } elseif (($step+$run)%2) {
                    $full=Invoke-Native (@('-c',$fullOut)+$fullArgs)
                    $inc=Invoke-Native @('-c',$incOut,'-d',$data,'--incremental')
                } else {
                    $inc=Invoke-Native @('-c',$incOut,'-d',$data,'--incremental')
                    $full=Invoke-Native (@('-c',$fullOut)+$fullArgs)
                }
                if ($inc.Output -notmatch 'output patched') { throw 'Repeated-output fast path was not used' }
                [NdsBench]::Compare($fullOut,$incOut)
                if ($OriginalExe) { [NdsBench]::Compare($originalOut,$incOut) }
                [NdsBench]::VerifyPreservedSections($romPath,$incOut)
                $fullTotal+=$full.Ms; $incTotal+=$inc.Ms
                if ($OriginalExe) { $originalTotal+=$originalResult.Ms }
                $rows.Add([pscustomobject]@{
                    Rom=[IO.Path]::GetFileNameWithoutExtension($romPath);Scenario=$scenario;Run=$run;Operations=$step
                    FullMs=$full.Ms;IncrementalMs=$inc.Ms;FullTotalMs=[math]::Round($fullTotal,2);IncrementalTotalMs=[math]::Round($incTotal,2)
                    OriginalMs=$(if ($OriginalExe) { $originalResult.Ms } else { $null })
                    OriginalTotalMs=$(if ($OriginalExe) { [math]::Round($originalTotal,2) } else { $null })
                    EditedPath=$(if ($scenario -eq 'Added') { '/' + [IO.Path]::GetRelativePath($data,$added[$step-1]).Replace('\','/') } else { $selected[$step-1].Key })
                    Stats=$inc.Output;Verification='PASS (all named files, direct bytes)'
                })
                $rows | ConvertTo-Json -Depth 5 | Set-Content "$OutputRoot/results.json"
            }
            $noop=Invoke-Native @('-c',$incOut,'-d',$data,'--incremental')
            if ($noop.Output -notmatch '0 patched bytes, output reused') { throw 'No-op touched the ROM' }
            Write-Output "$([IO.Path]::GetFileNameWithoutExtension($romPath)) / $scenario / run $run : 10 steps verified; original=$([math]::Round($originalTotal)) ms full=$([math]::Round($fullTotal)) ms incremental=$([math]::Round($incTotal)) ms"
        }
    }
}
$initializations | ConvertTo-Json | Set-Content "$OutputRoot/initialization.json"
$lines=@('# Incremental linking: repeated edit/build series','',
    "Windows native; $Runs series per ROM/scenario, median cumulative times. Each step edits one more distinct file and builds; N means N edits and N builds, not N edits in a single build.",
    'Initialization/export is measured separately in initialization.json. Every timed incremental build updates the previous state and the same output path. All builders see exactly the same inputs. Includes journal, output update and sync; Rust full writer does not explicitly sync. Warm OS cache; order alternates for two modes or rotates across three modes; no ROM hashes or boot tests.',
    'Add/grow: 64 KiB; remove: one middle file; shrink: half. Every step checks all named payload bytes against the full result.','',
    '| ROM | Edit | Operations/builds | Original total (ms) | Rust full total (ms) | Incremental total (ms) | Time saved vs original (or Rust full if omitted) |','|---|---|---:|---:|---:|---:|---:|')
foreach ($group in ($rows | Group-Object Rom,Scenario,Operations)) {
    $r=$group.Group[0]; $full=Median @($group.Group.FullTotalMs); $inc=Median @($group.Group.IncrementalTotalMs)
    $originalMedian=$(if ($OriginalExe) { Median @($group.Group.OriginalTotalMs) } else { $null })
    $baseline=$(if ($OriginalExe) { $originalMedian } else { $full })
    $saved=[math]::Round(100*(1-$inc/$baseline),1)
    $lines+="| $($r.Rom) | $($r.Scenario) | $($r.Operations) | $originalMedian | $full | **$inc** | $saved% |"
}
$lines | Set-Content "$OutputRoot/benchmark.md"
