# Runs bench.luau at High priority on one logical core, so other processes on a busy machine preempt it less.
param([string]$TrunkOut, [string]$HeadOut, [string]$Sections = "123456", [int]$Runs = 15, [string]$Out = "bench.txt", [long]$Core = 12)
$p = Start-Process -FilePath "lune" -ArgumentList @("run", "spike/ui/bench", $TrunkOut, $HeadOut, "$Runs", $Sections) -RedirectStandardOutput $Out -RedirectStandardError "$Out.err" -NoNewWindow -PassThru
$p.PriorityClass = "High"
$p.ProcessorAffinity = [IntPtr](1L -shl $Core)
$p.WaitForExit()
exit $p.ExitCode
