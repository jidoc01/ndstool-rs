param(
    [string]$InputRoot = 'D:\Work\ndstool-rs\compat-bench\benchmark\original',
    [string]$OriginalExe = 'C:\Users\jidoc\Downloads\ndstool.exe',
    [string]$RustExe = 'D:\Work\ndstool-rs\target\release\ndstool-rs.exe',
    [string]$OutputRoot = 'D:\Work\ndstool-rs\compat-bench\create-benchmark'
)

$ErrorActionPreference = 'Stop'

function Invoke-Creation([string]$Name, [string]$Exe, [bool]$Parallel) {
    $out = Join-Path $OutputRoot "$Name.nds"
    $arguments = @(
        '-c', $out,
        '-9', (Join-Path $InputRoot 'arm9.bin'),
        '-7', (Join-Path $InputRoot 'arm7.bin'),
        '-d', (Join-Path $InputRoot 'data'),
        '-t', (Join-Path $InputRoot 'banner.bin'),
        '-o', (Join-Path $InputRoot 'logo.bin')
    )
    if ($Parallel) { $arguments += '--parallel' }
    $times = @()
    1..3 | ForEach-Object {
        $run = $_
        $stdout = Join-Path $OutputRoot "$Name-$run.out.txt"
        $stderr = Join-Path $OutputRoot "$Name-$run.err.txt"
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $process = Start-Process -FilePath $Exe -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        $sw.Stop()
        if ($process.ExitCode -ne 0) { throw "$Name failed with exit code $($process.ExitCode)" }
        $times += [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
    }
    [pscustomobject]@{ Name = $Name; Path = $out; Times = $times; Median = ($times | Sort-Object)[1] }
}

New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
$runs = @(
    (Invoke-Creation 'original' $OriginalExe $false),
    (Invoke-Creation 'rust-sequential' $RustExe $false),
    (Invoke-Creation 'rust-parallel' $RustExe $true)
)

$sourceFiles = @(Get-ChildItem -LiteralPath (Join-Path $InputRoot 'data') -File -Recurse).Count
$rows = foreach ($run in $runs) {
    [pscustomobject]@{
        Tool = $run.Name
        MedianMs = $run.Median
        RunsMs = ($run.Times -join ', ')
        RomBytes = (Get-Item -LiteralPath $run.Path).Length
        DataFiles = $sourceFiles
    }
}

$lines = @(
    '# ndstool creation benchmark'
    ''
    "- Input tree: ``$InputRoot``"
    '- Runs: 3 per mode; median reported'
    '- All child processes run with hidden windows'
    ''
    '| Tool | Median (ms) | Runs (ms) | ROM bytes | Input data files |'
    '|---|---:|---|---:|---:|'
)
foreach ($row in $rows) {
    $lines += "| $($row.Tool) | $($row.MedianMs) | $($row.RunsMs) | $($row.RomBytes) | $($row.DataFiles) |"
}
$report = Join-Path $OutputRoot 'creation-benchmark.md'
$lines | Set-Content -LiteralPath $report -Encoding utf8
Write-Output $report
