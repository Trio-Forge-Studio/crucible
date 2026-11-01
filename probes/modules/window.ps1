param([int]$ProcId, [string]$Points = "", [string]$Out = "")
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;using System.Runtime.InteropServices;
public class PW{[DllImport("user32.dll")]public static extern bool PrintWindow(IntPtr h,IntPtr dc,uint f);[DllImport("user32.dll")]public static extern bool GetWindowRect(IntPtr h,out R r);public struct R{public int L,T,Rt,B;}}
"@
$p = Get-Process -Id $ProcId
$r = New-Object PW+R
[PW]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.Rt - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc()
[PW]::PrintWindow($p.MainWindowHandle, $dc, 2) | Out-Null
$g.ReleaseHdc($dc)
"size=$($bmp.Width)x$($bmp.Height)"
foreach ($point in ($Points -split ";" | Where-Object { $_ })) {
	$xy = $point -split ","
	$c = $bmp.GetPixel([int]$xy[0], [int]$xy[1])
	"$point=$($c.R),$($c.G),$($c.B)"
}
if ($Out) { $bmp.Save($Out) }
