param(
    [Parameter(Mandatory)][string]$Compiler,
    [Parameter(Mandatory)][string]$Fd,
    [Parameter(Mandatory)][string]$Report
)
$ErrorActionPreference = 'Stop'
$Compiler = (Resolve-Path -LiteralPath $Compiler).Path
$Fd = (Resolve-Path -LiteralPath $Fd).Path
$rows = @()
$edit = Join-Path $Fd 'src/core/Commands/shared/command-tree.ts'
$original = [IO.File]::ReadAllBytes($edit)
$runRoot = Join-Path $Fd ('build/compiler-budget-' + [guid]::NewGuid().ToString('N'))
try {
    for ($run = 0; $run -lt 5; $run++) {
        foreach ($place in @('warehouse', 'job')) {
            $out = Join-Path $runRoot "$run/$place"
            foreach ($scenario in @('full', 'edit')) {
                if ($scenario -eq 'edit') {
                    [IO.File]::AppendAllText($edit, "`n")
                }
                $info = [Diagnostics.ProcessStartInfo]::new()
                $info.FileName = $Compiler
                $info.WorkingDirectory = $Fd
                $info.UseShellExecute = $false
                $info.CreateNoWindow = $true
                $info.RedirectStandardOutput = $true
                $info.RedirectStandardError = $true
                $info.ArgumentList.Add((Join-Path $Fd "places/$place/tsconfig.ts7.json"))
                $info.ArgumentList.Add($out)
                $timer = [Diagnostics.Stopwatch]::StartNew()
                $process = [Diagnostics.Process]::Start($info)
                $stdout = $process.StandardOutput.ReadToEndAsync()
                $stderr = $process.StandardError.ReadToEndAsync()
                $process.WaitForExit()
                $timer.Stop()
                $output = $stdout.GetAwaiter().GetResult()
                $errors = $stderr.GetAwaiter().GetResult()
                $peak = $process.PeakWorkingSet64
                if ($process.ExitCode -ne 0) { throw $errors }
                $count = @(Get-ChildItem -LiteralPath $out -Recurse -Filter '*.luau').Count
                if ($count -eq 0) { throw 'No emitted modules' }
                $rows += [pscustomobject]@{
                    machine = $env:COMPUTERNAME; place = $place; run = $run; scenario = $scenario
                    wall_ms = $timer.Elapsed.TotalMilliseconds; peak_bytes = $peak
                    modules = $count; output = $output; errors = $errors
                }
                $process.Dispose()
                [IO.File]::WriteAllBytes($edit, $original)
            }
        }
    }
} finally {
    [IO.File]::WriteAllBytes($edit, $original)
}
$rows | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Report
$rows | Group-Object place,scenario | ForEach-Object {
    $times = @($_.Group.wall_ms | Sort-Object)
    $peaks = @($_.Group.peak_bytes | Sort-Object)
    [pscustomobject]@{ group = $_.Name; median_ms = $times[2]; min_ms = $times[0]; max_ms = $times[4]; peak_MB = $peaks[2] / 1e6 }
} | Format-Table
