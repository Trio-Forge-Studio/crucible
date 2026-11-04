# Runs bench.luau on three built runtimes at High priority on one logical core. Run from the repository root.
param([string]$TrunkOut, [string]$BaseOut, [string]$HeadOut, [string]$Sections = "12345678", [int]$Runs = 15, [string]$Out = "bench.txt", [long]$Core = 5)
$p = Start-Process -FilePath "lune" -ArgumentList @("run", "spike/ui/bench", $TrunkOut, $BaseOut, $HeadOut, "$Runs", $Sections) -RedirectStandardOutput $Out -RedirectStandardError "$Out.err" -NoNewWindow -PassThru
$p.PriorityClass = "High"
$p.ProcessorAffinity = [IntPtr](1L -shl $Core)
$p.WaitForExit()
exit $p.ExitCode
