$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path $projectRoot 'artifacts/fusion-check'
$fixtureData = Join-Path $fixtureRoot 'cn.codex.quota.widget'
$instance = [Guid]::NewGuid().ToString()
$className = "QuotaDockNativeWindow-$instance"
New-Item -ItemType Directory -Path $fixtureData -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $fixtureData 'widget-settings.json'), '{"version":1,"compactScale":0.46,"compactScaleVersion":2}', (New-Object Text.UTF8Encoding $false))
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class FusionProbe {
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int w, int h, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr monitor, ref Monitor info);
    [StructLayout(LayoutKind.Sequential)] public struct Monitor { public int Size; public Rect Bounds, Work; public uint Flags; }
    [DllImport("user32.dll")] public static extern bool KillTimer(IntPtr hwnd, IntPtr id);
    public delegate bool Callback(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(Callback callback, IntPtr data);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    public static IntPtr Find(uint wanted, string className) {
        IntPtr result=IntPtr.Zero;
        EnumWindows((hwnd,data)=>{
            uint pid; GetWindowThreadProcessId(hwnd,out pid);
            if(pid==wanted) { var text=new StringBuilder(256); GetClassName(hwnd,text,256);
                if(text.ToString()==className) { result=hwnd; return false; } }
            return true;
        },IntPtr.Zero);
        return result;
    }
    public static string Title(IntPtr hwnd) { var text=new StringBuilder(256); GetWindowText(hwnd,text,256); return text.ToString(); }
}
'@
[FusionProbe]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null


function Wait-For([scriptblock]$Condition, [string]$Label) {
    $deadline = (Get-Date).AddSeconds(5)
    do { if ($script:holdHover) { [FusionProbe]::PostMessage($script:window,0x200,[IntPtr]::Zero,[IntPtr]19660810) | Out-Null }; if (& $Condition) { return }; Start-Sleep -Milliseconds 30 } while ((Get-Date) -lt $deadline)
    throw "Timeout: $Label; rectangle=$(Get-Rect | ConvertTo-Json -Compress)"
}
function Send-Command([int]$Id) { [FusionProbe]::PostMessage($script:window, 0x111, [IntPtr]$Id, [IntPtr]::Zero) | Out-Null }
function Get-Rect { $r = New-Object FusionProbe+Rect; [FusionProbe]::GetWindowRect($script:window,[ref]$r) | Out-Null; return $r }
function Is-Size([int]$Width,[int]$Height) { $r=Get-Rect; return ($r.Right-$r.Left) -eq $Width -and ($r.Bottom-$r.Top) -eq $Height }
function Move-To([int]$X,[int]$Y) {
    [FusionProbe]::PostMessage($script:window,0x231,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
    Start-Sleep -Milliseconds 50
    [FusionProbe]::SetWindowPos($script:window,[IntPtr]::Zero,$X,$Y,0,0,0x15) | Out-Null
    [FusionProbe]::PostMessage($script:window,0x232,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
}
function Double-Click { [FusionProbe]::PostMessage($script:window,0x203,[IntPtr]::Zero,[IntPtr]655370) | Out-Null }
$launch = New-Object Diagnostics.ProcessStartInfo
$launch.FileName = Join-Path $projectRoot 'dist/QuotaDock.exe'
$launch.UseShellExecute = $false
$launch.EnvironmentVariables['APPDATA'] = $fixtureRoot
$launch.EnvironmentVariables['QUOTADOCK_TEST_INSTANCE'] = $instance
$process = [Diagnostics.Process]::Start($launch)
try {
    Wait-For { $script:window=[FusionProbe]::Find($process.Id,$className); $script:window -ne [IntPtr]::Zero } 'window'
    $dpi=[FusionProbe]::GetDpiForWindow($window)/96
    $wide=[int][Math]::Round(500*0.46*$dpi); $thin=[int][Math]::Round(28*0.46*$dpi)
    $hud=[int][Math]::Round(86*0.46*$dpi); $expanded=[int][Math]::Round(176*0.46*$dpi)
    $fullWidth=[int][Math]::Round(350*$dpi); $fullHeight=[int][Math]::Round(218*$dpi)
    $m=New-Object FusionProbe+Monitor; $m.Size=[Runtime.InteropServices.Marshal]::SizeOf($m)
    [FusionProbe]::GetMonitorInfo([FusionProbe]::MonitorFromWindow($window,2),[ref]$m) | Out-Null
    $work=$m.Work; $cx=[int](($work.Left+$work.Right-$wide)/2); $cy=[int](($work.Top+$work.Bottom-$wide)/2)
    Move-To $cx $cy
    Send-Command 108
    Wait-For { Is-Size $wide $hud } 'floating HUD'
    foreach ($edge in @('top','bottom','left','right')) {
        Move-To $cx $cy
        Wait-For { Is-Size $wide $hud } 'detach HUD'
        switch ($edge) {
            'top' { Move-To $cx ($work.Top+5) }
            'bottom' { Move-To $cx ($work.Bottom-$hud-5) }
            'left' { Move-To ($work.Left+5) $cy }
            'right' { Move-To ($work.Right-$wide-5) $cy }
        }
        if ($edge -in @('left','right')) { $w=$thin; $h=$wide } else { $w=$wide; $h=$thin }
        Wait-For { Is-Size $w $h } "$edge docking"
        $before=Get-Rect
        switch ($edge) {
            'top' { if ($before.Top -ne $work.Top) { throw 'Top anchor' } }
            'bottom' { if ($before.Bottom -ne $work.Bottom) { throw 'Bottom anchor' } }
            'left' { if ($before.Left -ne $work.Left) { throw 'Left anchor' } }
            'right' { if ($before.Right -ne $work.Right) { throw 'Right anchor' } }
        }
        Double-Click
        Wait-For { Is-Size $fullWidth $fullHeight } "$edge to full"
        Double-Click
        Wait-For { Is-Size $w $h } "$edge restored"
        $after=Get-Rect
        if ($before.Left -ne $after.Left -or $before.Top -ne $after.Top) { throw "$edge origin changed" }
        Write-Output "PASS: $edge docking; double click to full and back; original origin"
        if ($edge -eq 'right') {
            $script:holdHover=$true
            [FusionProbe]::PostMessage($window,0x200,[IntPtr]::Zero,[IntPtr]19660810) | Out-Null
            Wait-For { Is-Size $wide $expanded } 'side hover expands inward'
            $script:holdHover=$false
            $r=Get-Rect
            if ($r.Right -ne $work.Right) { throw 'Side hover escaped edge' }
            if ($r.Top -gt $before.Top+300 -or $r.Bottom -le $before.Top+300) { throw 'Side hover lost the pointer near the bottom' }
            Double-Click; Wait-For { Is-Size $fullWidth $fullHeight } 'hovered side to full'
            $r=Get-Rect
            if ($r.Right -gt $work.Right -or $r.Bottom -gt $work.Bottom) { throw 'Full window escaped the work area' }
            Double-Click; Wait-For { Is-Size $thin $wide } 'hovered side restored'
            $r=Get-Rect
            if ($r.Left -ne $before.Left -or $r.Top -ne $before.Top) { throw 'Hovered side origin was not restored' }
            Send-Command 305
            Wait-For { [FusionProbe]::Title($window) -like 'QuotaDock *7 *' } 'vertical island'
            Start-Sleep -Milliseconds 400
            if (-not (Is-Size $wide ([int][Math]::Round(146*0.46*$dpi)))) { throw 'Vertical notification width' }
            Send-Command 308
            Wait-For { [FusionProbe]::Title($window) -like 'QuotaDock *5 *' } 'second notice'
            Send-Command 308
            Wait-For { Is-Size $thin $wide } 'vertical restore after notice'
            Write-Output 'PASS: side hover expands inward; island notices restore vertical shape'
        }
    }
    Move-To $cx $cy
    Wait-For { Is-Size $wide $hud } 'detach final'
    Double-Click; Wait-For { Is-Size $fullWidth $fullHeight } 'floating full'
    Double-Click; Wait-For { Is-Size $wide $hud } 'floating restore'
    $saved=Get-Content (Join-Path $fixtureData 'widget-settings.json') -Raw | ConvertFrom-Json
    if (-not $saved.fusionMode -or $saved.dockEdge -ne 'none') { throw 'Fusion preference not saved' }
    Write-Output 'PASS: detached HUD; double click round trip; persisted fusion preference'
} finally {
    if ($script:window -ne [IntPtr]::Zero) { Send-Command 199 }
    if (-not $process.WaitForExit(3000)) { $process.Kill() }
}
