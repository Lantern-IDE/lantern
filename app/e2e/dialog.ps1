param([int]$ProcId, [string]$Click = "", [int]$Index = -1)
[Console]::OutputEncoding = [Text.Encoding]::UTF8
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class D {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  public static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
  public static string Txt(IntPtr h) { var s = new StringBuilder(4096); GetWindowText(h, s, 4096); return s.ToString(); }
  public static List<IntPtr> Top(uint pid) { var r = new List<IntPtr>(); EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h)) r.Add(h); return true; }, IntPtr.Zero); return r; }
  public static List<IntPtr> Kids(IntPtr w) { var r = new List<IntPtr>(); EnumChildWindows(w, (h, l) => { r.Add(h); return true; }, IntPtr.Zero); return r; }
}
"@
$found = $false
foreach ($w in [D]::Top($ProcId)) {
  if ([D]::Cls($w) -ne "#32770") { continue }
  $found = $true
  "대화상자: " + [D]::Txt($w)
  $bi = 0
  foreach ($k in [D]::Kids($w)) {
    $c = [D]::Cls($k); $t = [D]::Txt($k)
    if ($t) { "  [$c] $t" }
    if ($c -eq "Button") { if ($bi -eq $Index) { [D]::SendMessage($k, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; "  -> 클릭 #$bi" }; $bi++ }
    if ($Click -and $c -eq "Button" -and $t -like "*$Click*") { [D]::SendMessage($k, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; "  -> 클릭: $t" }
  }
}
if (-not $found) { "대화상자 없음" }
