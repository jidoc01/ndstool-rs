param(
    [Parameter(Mandatory)][string[]]$InputRoots,
    [Parameter(Mandatory)][string]$BeforeExe,
    [Parameter(Mandatory)][string]$OriginalExe,
    [string]$RustExe = "$PSScriptRoot/../target/release/ndstool-rs.exe",
    [string]$OutputRoot = "$PSScriptRoot/../compat-bench/full-creation",
    [int]$Runs = 5
)
$ErrorActionPreference = 'Stop'
if ($Runs -lt 1) { throw 'Runs must be positive' }
if ($env:NDSTOOL_PROFILE_CREATE) { throw 'Disable profiling for timed benchmarks' }
. "$PSScriptRoot/benchmark-incremental.ps1" -HelpersOnly -RustExe $RustExe -OutputRoot $OutputRoot -Runs $Runs
Add-Type -TypeDefinition @'
using System;
using System.IO;
public static class FullRomEquality {
    public static void Check(string a, string b) {
        using(var x=File.OpenRead(a)) using(var y=File.OpenRead(b)) {
            if(x.Length!=y.Length) throw new Exception("ROM sizes differ");
            var p=new byte[1048576]; var q=new byte[p.Length];
            while(x.Position<x.Length) {
                int n=(int)Math.Min(p.Length,x.Length-x.Position);
                x.ReadExactly(p.AsSpan(0,n)); y.ReadExactly(q.AsSpan(0,n));
                if(!p.AsSpan(0,n).SequenceEqual(q.AsSpan(0,n))) throw new Exception("ROM bytes differ");
            }
        }
    }
}
'@
$null=New-Item -ItemType Directory -Force $OutputRoot
$rows=[Collections.Generic.List[object]]::new()
$modes=@('Original','Before','Full','Parallel')
for ($i=0; $i -lt $InputRoots.Count; $i++) {
    $w=[IO.Path]::GetFullPath($InputRoots[$i])
    $common=@('-9',"$w/arm9.bin",'-7',"$w/arm7.bin",'-t',"$w/banner.bin",'-o',"$w/logo.bin",
        '-h',"$w/header.bin",'-d',"$w/data",'-y9',"$w/arm9ovr.bin",'-y7',"$w/arm7ovr.bin",'-y',"$w/overlays")
    $outputs=@{}
    foreach ($mode in $modes) { $outputs[$mode]=[IO.Path]::GetFullPath("$OutputRoot/rom-$($i+1)-$mode.nds") }
    # Warm each builder once, then rotate order to reduce cache/order bias.
    for ($run=0; $run -le $Runs; $run++) {
        for ($j=0; $j -lt $modes.Count; $j++) {
            $mode=$modes[($j+$run)%$modes.Count]
            $exe=switch($mode) { 'Original' { $OriginalExe }; 'Before' { $BeforeExe }; default { $RustExe } }
            $arguments=@('-c',$outputs[$mode])+$common
            if ($mode -eq 'Parallel') { $arguments+=@('--parallel') }
            $result=Invoke-Native $arguments -Executable $exe
            if ($run -gt 0) { $rows.Add([pscustomobject]@{Rom=$i+1;Mode=$mode;Run=$run;Ms=$result.Ms;Bytes=(Get-Item $outputs[$mode]).Length}) }
        }
        # Outside timed regions. No hashes, and no ROM content in reports.
        [FullRomEquality]::Check($outputs.Before,$outputs.Full)
        [FullRomEquality]::Check($outputs.Before,$outputs.Parallel)
        $null=[NdsBench]::Compare($outputs.Original,$outputs.Full)
    }
    Write-Output "ROM $($i+1): $Runs timed runs/mode; full bytes and original payloads verified"
    $rows | ConvertTo-Json | Set-Content "$OutputRoot/results.json"
}
$summary=foreach ($i in 1..$InputRoots.Count) {
    $r=[ordered]@{Rom=$i}
    foreach ($mode in $modes) { $r[$mode]=Median @($rows | Where-Object { $_.Rom -eq $i -and $_.Mode -eq $mode } | ForEach-Object Ms) }
    $r.SavedVsBefore=[math]::Round(100*(1-$r.Full/$r.Before),1)
    [pscustomobject]$r
}
$summary | ConvertTo-Json | Set-Content "$OutputRoot/summary.json"
$summary | Format-Table
