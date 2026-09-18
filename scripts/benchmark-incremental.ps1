#requires -Version 7.0
param(
    [string[]]$Roms,
    [string]$RustExe = (Join-Path $PSScriptRoot '../target/release/ndstool-rs.exe'),
    [string]$OutputRoot = (Join-Path $PSScriptRoot '../compat-bench/incremental-scenarios'),
    [int]$Runs = 3,
    [switch]$HelpersOnly
)
$ErrorActionPreference = 'Stop'
if ($Runs -lt 1) { throw 'Runs must be positive' }
$RustExe = (Resolve-Path -LiteralPath $RustExe).Path

# Direct byte comparisons, not content hashing. Kept outside timed regions.
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Collections.Generic;
using System.Text;
public static class NdsBench {
    public sealed class Range { public uint Id, Start, End; public long Size => (long)End-Start; }
    static uint U32(byte[] b,int p) => BitConverter.ToUInt32(b,p);
    static ushort U16(byte[] b,int p) => BitConverter.ToUInt16(b,p);
    public static byte[] Read(string path,long offset,int size) {
        using var f=File.OpenRead(path); f.Position=offset;
        var b=new byte[size]; f.ReadExactly(b); return b;
    }
    public static Dictionary<string,Range> Index(string path) {
        var h=Read(path,0,512);
        var fnt=Read(path,U32(h,0x40),(int)U32(h,0x44));
        var fat=Read(path,U32(h,0x48),(int)U32(h,0x4c));
        var result=new Dictionary<string,Range>(); var seen=new HashSet<int>();
        long length=new FileInfo(path).Length;
        void Walk(int id,string prefix) {
            if(!seen.Add(id)) throw new Exception("FNT cycle");
            int p=(id&4095)*8; int at=(int)U32(fnt,p); uint fileId=U16(fnt,p+4);
            while(fnt[at]!=0) {
                byte flag=fnt[at++]; int n=flag&127;
                string name=Encoding.UTF8.GetString(fnt,at,n); at+=n;
                if((flag&128)!=0) {int child=U16(fnt,at);at+=2;Walk(child,prefix+name+"/");}
                else {
                    int slot=checked((int)fileId*8);
                    uint start=U32(fat,slot), end=U32(fat,slot+4);
                    if(start>end||end>length) throw new Exception("FAT outside ROM");
                    result.Add(prefix+name,new Range{Id=fileId++,Start=start,End=end});
                }
            }
        }
        Walk(0xf000,"/"); return result;
    }
    public static void Compare(string a,string b) {
        var ia=Index(a);var ib=Index(b);
        if(ia.Count!=ib.Count)throw new Exception("File count mismatch");
        using var fa=File.OpenRead(a);using var fb=File.OpenRead(b);
        var ba=new byte[65536];var bb=new byte[65536];
        foreach(var kv in ia) {
            if(!ib.TryGetValue(kv.Key,out var y)||kv.Value.Size!=y.Size)throw new Exception("Path/size mismatch: "+kv.Key);
            fa.Position=kv.Value.Start;fb.Position=y.Start;long left=y.Size;
            while(left>0) {
                int n=(int)Math.Min(left,ba.Length);
                fa.ReadExactly(ba,0,n);fb.ReadExactly(bb,0,n);
                if(!ba.AsSpan(0,n).SequenceEqual(bb.AsSpan(0,n)))throw new Exception("Payload mismatch: "+kv.Key);
                left-=n;
            }
        }
    }
    public static void VerifyPreservedSections(string original,string edited) {
        var a=Read(original,0,512);var b=Read(edited,0,512);
        void Same(uint start,uint size) {
            using var fa=File.OpenRead(original);using var fb=File.OpenRead(edited);
            fa.Position=start;fb.Position=start;
            var ba=new byte[65536];var bb=new byte[65536];long left=size;
            while(left>0) {
                int n=(int)Math.Min(left,ba.Length);fa.ReadExactly(ba,0,n);fb.ReadExactly(bb,0,n);
                if(!ba.AsSpan(0,n).SequenceEqual(bb.AsSpan(0,n)))throw new Exception("Preserved section changed at "+start);
                left-=n;
            }
        }
        var fatA=Read(original,U32(a,0x48),(int)U32(a,0x4c));
        var fatB=Read(edited,U32(b,0x48),(int)U32(b,0x4c));
        foreach(var fields in new[]{(0x20,0x2c),(0x30,0x3c),(0x50,0x54),(0x58,0x5c)}) {
            uint start=U32(a,fields.Item1),size=U32(a,fields.Item2);
            if(start!=U32(b,fields.Item1)||size!=U32(b,fields.Item2))throw new Exception("Binary section layout changed");
            if(size==0)continue;
            Same(start,size);
            if(fields.Item1==0x50||fields.Item1==0x58) {
                var table=Read(original,start,(int)size);
                if(size%32!=0)throw new Exception("Invalid overlay table");
                for(int at=0;at<table.Length;at+=32) {
                    int slot=checked((int)U32(table,at+24)*8);
                    uint s=U32(fatA,slot),e=U32(fatA,slot+4);
                    if(s!=U32(fatB,slot)||e!=U32(fatB,slot+4))throw new Exception("Overlay FAT changed");
                    Same(s,e-s);
                }
            }
        }
        Same(0xc0,0x9e); // Logo and its checksum, excluding mutable header CRC.
        ushort crc=0xffff;
        for(int i=0;i<0x15e;i++) {
            crc^=b[i];
            for(int j=0;j<8;j++)crc=(ushort)((crc>>1)^((crc&1)!=0?0xa001:0));
        }
        if(crc!=U16(b,0x15e))throw new Exception("Invalid header CRC");
    }
}
'@
function Invoke-Native([string[]]$Arguments, [string]$Executable = $RustExe) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($arg in $Arguments) { $info.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $null = $process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $timer.Stop()
    $output = $stdout.GetAwaiter().GetResult()
    $errors = $stderr.GetAwaiter().GetResult()
    $code = $process.ExitCode
    $process.Dispose()
    if ($code -ne 0) { throw "$Executable failed: $errors" }
    [pscustomobject]@{ Ms = [math]::Round($timer.Elapsed.TotalMilliseconds,2); Output = $output.Trim(); Stderr = $errors.Trim() }
}
function Median($Values) {
    $sorted = @($Values | Sort-Object)
    if ($sorted.Count % 2) { $sorted[[int][math]::Floor($sorted.Count/2)] }
    else { ($sorted[$sorted.Count/2-1]+$sorted[$sorted.Count/2])/2 }
}
if ($HelpersOnly) { return }
if (!$Roms) { throw 'Supply -Roms' }
if (Test-Path -LiteralPath $OutputRoot) { throw 'Use a new output directory to keep previous experiments intact.' }
New-Item -ItemType Directory -Path $OutputRoot | Out-Null
$OutputRoot = (Resolve-Path -LiteralPath $OutputRoot).Path
$rows = [Collections.Generic.List[object]]::new()
$romNumber = 0
foreach ($romPath in $Roms) {
    $romPath = (Resolve-Path -LiteralPath $romPath).Path
    $romNumber++
    $work = Join-Path $OutputRoot "rom-$romNumber"
    $data = Join-Path $work 'data'
    New-Item -ItemType Directory -Path $data | Out-Null
    $extraction = @('-x',$romPath,'-d',$data,'-9',"$work/arm9.bin",'-7',"$work/arm7.bin",
        '-t',"$work/banner.bin",'-o',"$work/logo.bin",'-y9',"$work/arm9ovr.bin",'-y7',"$work/arm7ovr.bin",
        '-y',"$work/overlays",'--parallel','--incremental')
    $extractResult = Invoke-Native $extraction
    $extractResult | ConvertTo-Json | Set-Content "$work/extraction.json"
    [IO.File]::WriteAllBytes("$work/header.bin",[NdsBench]::Read($romPath,0,512))
    $cache = Join-Path $data '.ndstool-rs'
    $stateBytes = [IO.File]::ReadAllBytes("$cache/layout.bin")
    $snapshot = @(Get-ChildItem -LiteralPath $cache -Filter 'snapshot-*.nds')[0]
    $savedSnapshot = Join-Path $work 'baseline.nds'
    [IO.File]::Copy($snapshot.FullName,$savedSnapshot)
    $index = [NdsBench]::Index($romPath)
    $byOffset = @($index.GetEnumerator() | Where-Object { $_.Value.Size -ge 2 } | Sort-Object { $_.Value.Start })
    $selected = $byOffset[[int][math]::Floor($byOffset.Count/2)]
    $selectedPath = Join-Path $data $selected.Key.TrimStart('/')
    $original = [IO.File]::ReadAllBytes($selectedPath)
    $originalTime = [IO.File]::GetLastWriteTimeUtc($selectedPath)
    $addedPath = Join-Path (Split-Path -Parent $selectedPath) '__ndstool_benchmark_added.bin'
    if (Test-Path -LiteralPath $addedPath) { throw 'Benchmark filename collision' }
    $fullArgs = @('-9',"$work/arm9.bin",'-7',"$work/arm7.bin",'-t',"$work/banner.bin",
        '-o',"$work/logo.bin",'-h',"$work/header.bin",'-d',$data,
        '-y9',"$work/arm9ovr.bin",'-y7',"$work/arm7ovr.bin",'-y',"$work/overlays")
    # One untimed warm-up, same full options including both overlay tables.
    $null = Invoke-Native (@('-c',"$work/warmup.nds") + $fullArgs)
    foreach ($scenario in @('Added','Removed','Grown','Shrunk')) {
        $fullTimes = @(); $incTimes = @(); $incStats = @()
        for ($run=1; $run -le $Runs; $run++) {
            # Restoring the baseline is outside timing: every run measures a
            # real edit, never a no-op against the previous measured output.
            [IO.File]::WriteAllBytes($selectedPath,$original)
            [IO.File]::SetLastWriteTimeUtc($selectedPath,$originalTime)
            if ([IO.File]::Exists($addedPath)) { [IO.File]::Delete($addedPath) }
            [IO.File]::Copy($savedSnapshot,$snapshot.FullName,$true)
            [IO.File]::WriteAllBytes("$cache/layout.bin",$stateBytes)
            switch ($scenario) {
                'Added' { [IO.File]::WriteAllBytes($addedPath,[byte[]]::new(65536)) }
                'Removed' { [IO.File]::Delete($selectedPath) }
                'Grown' {
                    $grown = [byte[]]::new($original.Length+65536)
                    [Array]::Copy($original,$grown,$original.Length)
                    [IO.File]::WriteAllBytes($selectedPath,$grown)
                }
                'Shrunk' { [IO.File]::WriteAllBytes($selectedPath,$original[0..([int][math]::Floor($original.Length/2)-1)]) }
            }
            $fullOut = Join-Path $work "$scenario-full.nds"
            $incOut = Join-Path $work "$scenario-incremental.nds"
            $incArgs = @('-c',$incOut,'-d',$data,'--incremental')
            # Alternate order to reduce systematic warm-cache bias.
            if ($run % 2) {
                $fullResult = Invoke-Native (@('-c',$fullOut) + $fullArgs)
                $incResult = Invoke-Native $incArgs
            } else {
                $incResult = Invoke-Native $incArgs
                $fullResult = Invoke-Native (@('-c',$fullOut) + $fullArgs)
            }
            $fullTimes += $fullResult.Ms; $incTimes += $incResult.Ms; $incStats += $incResult.Output
        }
        [NdsBench]::Compare($fullOut,$incOut)
        # Verify state advances correctly: no input changes on the next build.
        $repeatOut = Join-Path $work 'repeat.nds'
        $repeat = Invoke-Native @('-c',$repeatOut,'-d',$data,'--incremental')
        if ($repeat.Output -notmatch '0 changed/new, 0 deleted') { throw 'State did not advance' }
        [NdsBench]::Compare($incOut,$repeatOut)
        $fullMedian = Median $fullTimes; $incMedian = Median $incTimes
        $row = [pscustomobject]@{
            Rom = [IO.Path]::GetFileNameWithoutExtension($romPath); Scenario = $scenario
            EditedPath = $selected.Key; OriginalBytes = $original.Length
            FullMs = $fullMedian; IncrementalMs = $incMedian
            ReductionPercent = [math]::Round(100*(1-$incMedian/$fullMedian),1)
            FullRunsMs = $fullTimes; IncrementalRunsMs = $incTimes; Stats = $incStats
            PayloadComparison = 'PASS (all named files, direct bytes)'; StateAdvance = 'PASS'
        }
        $rows.Add($row)
        $rows | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $OutputRoot 'results.json')
        Write-Output "$($row.Rom) / $scenario : full=$fullMedian ms incremental=$incMedian ms; verified"
    }
    [IO.File]::WriteAllBytes($selectedPath,$original)
    [IO.File]::SetLastWriteTimeUtc($selectedPath,$originalTime)
    if ([IO.File]::Exists($addedPath)) { [IO.File]::Delete($addedPath) }
}
$lines = @('# Incremental creation benchmark','',
    'Windows native release build; three runs, median milliseconds; warm OS cache; single-worker creation.',
    'Timings include process startup, output publication and incremental snapshot/state update. Baseline reset and byte comparisons are outside timing.',
    'One file near the middle of physical payload order is edited. Add: 64 KiB in its directory; remove: selected file; grow: +64 KiB; shrink: half size.',
    'Incremental deletion/shrinking leaves holes. Insertion/removal can reassign IDs within the edited directory. Game boot is not tested.','',
    '| ROM | Scenario | Full build | Incremental | Time reduction |',
    '|---|---|---:|---:|---:|')
foreach ($row in $rows) { $lines += "| $($row.Rom) | $($row.Scenario) | $($row.FullMs) | $($row.IncrementalMs) | $($row.ReductionPercent)% |" }
$lines | Set-Content (Join-Path $OutputRoot 'benchmark.md')
