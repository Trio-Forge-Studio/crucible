# Runs ab.luau at High priority on one logical core. Subjects are name=out pairs, comma separated, such as
# "l3a=C:/rt/l3a,l4=C:/rt/l4". Run from the repository root.
param([string]$Subjects, [int]$Runs = 31, [string]$Out = "ab.txt", [long]$Core = 5)
$p = Start-Process -FilePath "lune" -ArgumentList (@("run", "spike/ui/ab", "$Runs") + ($Subjects -split ",")) -RedirectStandardOutput $Out -RedirectStandardError "$Out.err" -NoNewWindow -PassThru
$p.PriorityClass = "High"
$p.ProcessorAffinity = [IntPtr](1L -shl $Core)
$p.WaitForExit()
exit $p.ExitCode
