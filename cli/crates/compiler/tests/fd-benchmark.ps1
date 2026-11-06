param(
    [Parameter(Mandatory)][string]$Crucible,
    [Parameter(Mandatory)][string]$Fd,
    [Parameter(Mandatory)][string]$Report,
    [int]$Rounds = 5
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class PeakMemory {
    [StructLayout(LayoutKind.Sequential)]
    public struct Counters {
        public uint cb, PageFaultCount;
        public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage, QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage;
    }
    [DllImport("psapi.dll", SetLastError = true)]
    public static extern bool GetProcessMemoryInfo(IntPtr process, ref Counters counters, uint size);
    public static ulong Read(IntPtr handle) {
        Counters c = new Counters();
        c.cb = (uint)Marshal.SizeOf<Counters>();
        if (!GetProcessMemoryInfo(handle, ref c, c.cb)) throw new System.ComponentModel.Win32Exception();
        return c.PeakWorkingSetSize.ToUInt64();
    }
}
'@
function Invoke-Timed([string[]]$Arguments) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Crucible
    $info.WorkingDirectory = $Fd
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    # The __compile child exits when its stdin closes, because the build ties it to its parent that way.
    $info.RedirectStandardInput = $true
    foreach ($a in $Arguments) { $info.ArgumentList.Add($a) }
    $cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $p = [Diagnostics.Process]::Start($info)
    $h = $p.Handle
    $out = $p.StandardOutput.ReadToEndAsync()
    $err = $p.StandardError.ReadToEndAsync()
    $p.WaitForExit()
    $timer.Stop()
    $result = [pscustomobject]@{
        exit = $p.ExitCode; wall_ms = $timer.Elapsed.TotalMilliseconds; peak_bytes = [PeakMemory]::Read($h)
        cpu_before = $cpu; stdout_bytes = $out.GetAwaiter().GetResult().Length; stderr = $err.GetAwaiter().GetResult()
    }
    $p.Dispose()
    if ($result.exit -ne 0) { throw "crucible $($Arguments -join ' ') exited $($result.exit): $($result.stderr)" }
    $result
}
$edit = Join-Path $Fd 'src/core/Commands/shared/command-tree.ts'
$original = [IO.File]::ReadAllBytes($edit)
$rows = @()
try {
    for ($round = 1; $round -le $Rounds; $round++) {
        foreach ($place in @('warehouse', 'job')) {
            $out = Join-Path $Fd "places/$place/out"
            $rbxl = "build/bench/$place.rbxl"
            Get-ChildItem -LiteralPath $out -Recurse -Filter '*.luau' -ErrorAction SilentlyContinue | Remove-Item -Force
            $full = Invoke-Timed @('build', '--place', $place, '--output', $rbxl)
            $modules = @(Get-ChildItem -LiteralPath $out -Recurse -Filter '*.luau').Count
            if ($modules -eq 0) { throw "no modules written for $place" }
            [IO.File]::AppendAllText($edit, "`n")
            try { $edited = Invoke-Timed @('build', '--place', $place, '--output', $rbxl) } finally { [IO.File]::WriteAllBytes($edit, $original) }
            $again = Invoke-Timed @('build', '--place', $place, '--output', $rbxl)
            $compile = Invoke-Timed @('__compile', "tsconfig.$place.json", "$place.generated.project.json", "places/$place/include")
            foreach ($pair in @(@('full', $full), @('edit', $edited), @('nochange', $again), @('compile_child', $compile))) {
                $rows += [pscustomobject]@{ place = $place; round = $round; scenario = $pair[0]; wall_ms = $pair[1].wall_ms; peak_bytes = $pair[1].peak_bytes; cpu_before = $pair[1].cpu_before; modules = $modules; stdout_bytes = $pair[1].stdout_bytes }
            }
        }
    }
} finally { [IO.File]::WriteAllBytes($edit, $original) }
$rows | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $Report
$rows | Group-Object place, scenario | ForEach-Object {
    $t = @($_.Group.wall_ms | Sort-Object); $m = @($_.Group.peak_bytes | Sort-Object); $c = @($_.Group.cpu_before | Sort-Object)
    [pscustomobject]@{ group = $_.Name; n = $t.Count; median_ms = [math]::Round($t[[int][math]::Floor($t.Count / 2)]); min_ms = [math]::Round($t[0]); max_ms = [math]::Round($t[-1]); peak_MB = [math]::Round($m[-1] / 1e6, 1); cpu_max = $c[-1]; modules = $_.Group[0].modules }
} | Format-Table -AutoSize
