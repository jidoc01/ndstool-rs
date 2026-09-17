param(
    [Parameter(Mandatory = $true)][string]$Rom,
    [string]$OutputRoot = 'D:\Work\ndstool-rs\compat-bench\timing-only',
    [int]$Runs = 3
)

$ErrorActionPreference = 'Stop'

function Quote-ProcessArgument([string]$Argument) {
    if ($Argument -match '[\s"]') { '"' + $Argument.Replace('"', '\"') + '"' } else { $Argument }
}

function Measure-Mode([string]$Name, [string]$Exe, [bool]$Parallel) {
    $out = Join-Path $OutputRoot $Name
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $bannerOption = if ($Name -eq 'original') { '-t' } else { '-b' }
    $arguments = @('-x', $Rom, '-d', (Join-Path $out 'data'), '-9', (Join-Path $out 'arm9.bin'), '-7', (Join-Path $out 'arm7.bin'), $bannerOption, (Join-Path $out 'banner.bin'), '-o', (Join-Path $out 'logo.bin'), '-y', (Join-Path $out 'overlays'))
    if ($Parallel) { $arguments += '--parallel' }
    $times = @()
    1..$Runs | ForEach-Object {
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $quoted = @($arguments | ForEach-Object { Quote-ProcessArgument $_ })
        $process = Start-Process -FilePath $Exe -ArgumentList $quoted -WindowStyle Hidden -Wait -PassThru `
            -RedirectStandardOutput (Join-Path $out "run-$_.out.txt") -RedirectStandardError (Join-Path $out "run-$_.err.txt")
        $sw.Stop()
        if ($process.ExitCode -ne 0) { throw "$Name failed with exit code $($process.ExitCode)" }
        $times += [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
    }
    [pscustomobject]@{ Tool = $Name; RunsMs = ($times -join ', '); MedianMs = ($times | Sort-Object)[[math]::Floor($times.Count / 2)] }
}

New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
$rows = @(
    (Measure-Mode 'original' 'C:\Users\jidoc\Downloads\ndstool.exe' $false),
    (Measure-Mode 'rust-sequential' 'D:\Work\ndstool-rs\target\release\ndstool-rs.exe' $false),
    (Measure-Mode 'rust-parallel' 'D:\Work\ndstool-rs\target\release\ndstool-rs.exe' $true)
)
$lines = @('# ndstool extraction timing benchmark', '', "- ROM: ``$Rom``", "- Runs: $Runs per mode; no content hash comparison", '', '| Tool | Median (ms) | Runs (ms) |', '|---|---:|---|')
foreach ($row in $rows) { $lines += "| $($row.Tool) | $($row.MedianMs) | $($row.RunsMs) |" }
$report = Join-Path $OutputRoot 'timing-only.md'
$lines | Set-Content -LiteralPath $report -Encoding utf8
$rows | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $OutputRoot 'timing.json') -Encoding utf8
Write-Output $report
