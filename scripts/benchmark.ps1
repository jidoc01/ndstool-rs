param(
    [string]$Rom = 'D:\Work\ds-patcher\template\baserom.nds',
    [string]$OriginalExe = 'C:\Users\jidoc\Downloads\ndstool.exe',
    [string]$RustExe = 'D:\Work\ndstool-rs\target\release\ndstool-rs.exe',
    [string]$OutputRoot = 'D:\Work\ndstool-rs\compat-bench\benchmark'
)

$ErrorActionPreference = 'Stop'

function Hash-File([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Snapshot-Tree([string]$Root) {
    $snapshot = @{}
    if (Test-Path -LiteralPath $Root) {
        Get-ChildItem -LiteralPath $Root -File -Recurse | ForEach-Object {
            $relative = $_.FullName.Substring($Root.Length).TrimStart('\')
            $snapshot[$relative] = Hash-File $_.FullName
        }
    }
    $snapshot
}

function Compare-Tree([string]$Left, [string]$Right) {
    $a = Snapshot-Tree $Left
    $b = Snapshot-Tree $Right
    $keys = @($a.Keys + $b.Keys | Sort-Object -Unique)
    $missing = 0; $extra = 0; $different = 0
    foreach ($key in $keys) {
        if (-not $b.ContainsKey($key)) { $missing++ }
        elseif (-not $a.ContainsKey($key)) { $extra++ }
        elseif ($a[$key] -ne $b[$key]) { $different++ }
    }
    [pscustomobject]@{ Missing = $missing; Extra = $extra; Different = $different }
}

function Quote-ProcessArgument([string]$Argument) {
    if ($Argument -match '[\s"]') {
        '"' + $Argument.Replace('"', '\"') + '"'
    } else {
        $Argument
    }
}

function Invoke-Extraction([string]$Name, [string]$Exe, [bool]$Parallel) {
    $out = Join-Path $OutputRoot $Name
    New-Item -ItemType Directory -Force -Path $out | Out-Null
    $bannerOption = if ($Name -eq 'original') { '-t' } else { '-b' }
    $arguments = @('-x', $Rom, '-d', (Join-Path $out 'data'), '-9', (Join-Path $out 'arm9.bin'), '-7', (Join-Path $out 'arm7.bin'), $bannerOption, (Join-Path $out 'banner.bin'), '-o', (Join-Path $out 'logo.bin'), '-y', (Join-Path $out 'overlays'))
    if ($Parallel) { $arguments += '--parallel' }
    $times = @()
    1..3 | ForEach-Object {
        $runNumber = $_
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $stdout = Join-Path $out "run-$runNumber.out.txt"
        $stderr = Join-Path $out "run-$runNumber.err.txt"
        $quotedArguments = @($arguments | ForEach-Object { Quote-ProcessArgument $_ })
        $process = Start-Process -FilePath $Exe -ArgumentList $quotedArguments -WindowStyle Hidden -Wait -PassThru `
            -RedirectStandardOutput $stdout -RedirectStandardError $stderr
        $exitCode = $process.ExitCode
        $sw.Stop()
        if ($exitCode -ne 0) { throw "$Name failed with exit code $exitCode" }
        $times += [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
    }
    [pscustomobject]@{ Name = $Name; Path = $out; Times = $times; Median = ($times | Sort-Object)[1] }
}

New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
$runs = @(
    (Invoke-Extraction 'original' $OriginalExe $false),
    (Invoke-Extraction 'rust-sequential' $RustExe $false),
    (Invoke-Extraction 'rust-parallel' $RustExe $true)
)
$baseline = $runs[0]
$rows = foreach ($run in $runs) {
    $data = Compare-Tree (Join-Path $baseline.Path 'data') (Join-Path $run.Path 'data')
    $overlays = Compare-Tree (Join-Path $baseline.Path 'overlays') (Join-Path $run.Path 'overlays')
    $arm7 = (Hash-File (Join-Path $baseline.Path 'arm7.bin')) -eq (Hash-File (Join-Path $run.Path 'arm7.bin'))
    $banner = (Hash-File (Join-Path $baseline.Path 'banner.bin')) -eq (Hash-File (Join-Path $run.Path 'banner.bin'))
    $logo = (Hash-File (Join-Path $baseline.Path 'logo.bin')) -eq (Hash-File (Join-Path $run.Path 'logo.bin'))
    [pscustomobject]@{
        Tool = $run.Name
        MedianMs = $run.Median
        RunsMs = ($run.Times -join ', ')
        DataFiles = (Snapshot-Tree (Join-Path $run.Path 'data')).Count
        DataDiff = "$($data.Missing)/$($data.Extra)/$($data.Different)"
        OverlayDiff = "$($overlays.Missing)/$($overlays.Extra)/$($overlays.Different)"
        ARM7 = $arm7
        Banner = $banner
        Logo = $logo
    }
}

$lines = @(
    '# ndstool compatibility benchmark'
    ''
    "- ROM: ``$Rom``"
    '- Runs: 3 per mode; median reported'
    '- DataDiff/OverlayDiff format: missing/extra/different compared with original'
    ''
    '| Tool | Median (ms) | Runs (ms) | Data files | Data diff | Overlay diff | ARM7 | Banner | Logo |'
    '|---|---:|---|---:|---|---|---|---|---|'
)
foreach ($row in $rows) {
    $lines += "| $($row.Tool) | $($row.MedianMs) | $($row.RunsMs) | $($row.DataFiles) | $($row.DataDiff) | $($row.OverlayDiff) | $($row.ARM7) | $($row.Banner) | $($row.Logo) |"
}
$report = Join-Path $OutputRoot 'benchmark.md'
$lines | Set-Content -LiteralPath $report -Encoding utf8
Write-Output $report
