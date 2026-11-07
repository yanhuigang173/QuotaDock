$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $projectRoot 'dist/QuotaDock.exe'
$fixtureRoot = Join-Path $projectRoot 'artifacts/notification-check'
$fixtureData = Join-Path $fixtureRoot 'cn.codex.quota.widget'
New-Item -ItemType Directory -Path $fixtureData -Force | Out-Null
# An existing settings file without style must migrate to the island default.
[System.IO.File]::WriteAllText((Join-Path $fixtureData 'widget-settings.json'), '{"version":1,"resetAlerts":{"enabled":true},"trayIconStep":3,"compactScale":0.72}', (New-Object System.Text.UTF8Encoding $false))

Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NoticeProbe {
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
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
[NoticeProbe]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null

function Wait-Condition([scriptblock]$Condition, [string]$Description, [int]$TimeoutSeconds = 8) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        if ($script:expectedWidth -and $script:mainWindow -ne [IntPtr]::Zero) {
            $rect = New-Object NoticeProbe+Rect
            [NoticeProbe]::GetWindowRect($script:mainWindow, [ref]$rect) | Out-Null
            if (($rect.Right - $rect.Left) -ne $script:expectedWidth) { throw 'Width changed during transition' }
        }
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    throw "Timed out: $Description"
}
function Send-Command([int]$Id) {
    if (-not [NoticeProbe]::PostMessage($script:mainWindow, 0x111, [IntPtr]$Id, [IntPtr]::Zero)) { throw "Cannot send command $Id" }
}
function Get-Size {
    $rect = New-Object NoticeProbe+Rect
    [NoticeProbe]::GetWindowRect($script:mainWindow, [ref]$rect) | Out-Null
    return @(($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
}

$launch = New-Object System.Diagnostics.ProcessStartInfo
$launch.FileName = $exe
$launch.Arguments = '--preview-alerts'
$launch.UseShellExecute = $false
$launch.EnvironmentVariables['APPDATA'] = $fixtureRoot
$process = [System.Diagnostics.Process]::Start($launch)
try {
    Wait-Condition { $script:mainWindow = [NoticeProbe]::Find($process.Id, 'QuotaDockNativeWindow'); $script:mainWindow -ne [IntPtr]::Zero } 'main window'
    Wait-Condition { [NoticeProbe]::Title($script:mainWindow) -like 'QuotaDock *7 *' } 'weekly island notice'
    $script:expectedWidth = (Get-Size)[0]
    $originalWidth = [Math]::Round(500 * 0.46 * [NoticeProbe]::GetDpiForWindow($mainWindow) / 96)
    if ($script:expectedWidth -ne $originalWidth) { throw "Original default width was not restored: actual=$script:expectedWidth expected=$originalWidth dpi=$([NoticeProbe]::GetDpiForWindow($mainWindow))" }
    $heights = @()
    for ($sample = 0; $sample -lt 24; $sample++) {
        $size = Get-Size
        if ($size[0] -ne $script:expectedWidth) { throw 'Opening animation changed width' }
        $heights += $size[1]
        Start-Sleep -Milliseconds 16
    }
    if (@($heights | Select-Object -Unique).Count -lt 3) { throw 'Opening animation did not produce intermediate heights' }
    if (-not [NoticeProbe]::IsWindowVisible($mainWindow)) { throw 'Island notice is hidden' }
    Wait-Condition { [NoticeProbe]::Title($mainWindow) -like 'QuotaDock *5 *' } 'hourly island notice after automatic expiry' 13
    Wait-Condition { $size = Get-Size; [NoticeProbe]::Title($mainWindow) -eq 'QuotaDock' -and $size[1] -eq [Math]::Round($size[0] * 86 / 500) } 'automatic restoration' 13
    $baseSize = Get-Size
    Write-Output 'PASS: default island; 7d then 5h; automatic restoration'

    Send-Command 306
    Send-Command 305
    Wait-Condition { $script:dialog = [NoticeProbe]::Find($process.Id, '#32770'); [NoticeProbe]::Title($script:dialog) -like 'QuotaDock *7 *' } 'weekly native dialog'
    [NoticeProbe]::PostMessage($dialog, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Wait-Condition { $script:dialog = [NoticeProbe]::Find($process.Id, '#32770'); [NoticeProbe]::Title($script:dialog) -like 'QuotaDock *5 *' } 'hourly native dialog'
    [NoticeProbe]::PostMessage($dialog, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Wait-Condition { [NoticeProbe]::Find($process.Id, '#32770') -eq [IntPtr]::Zero } 'native dialog dismissal'
    $settings = Get-Content -LiteralPath (Join-Path $fixtureData 'widget-settings.json') -Raw | ConvertFrom-Json
    if ($settings.resetAlerts.style -ne 'popup') { throw 'Popup style was not persisted' }
    Write-Output 'PASS: popup style; both dialogs; persisted selection'

    Send-Command 307
    Send-Command 100
    Wait-Condition { -not [NoticeProbe]::IsWindowVisible($mainWindow) } 'hidden HUD'
    Send-Command 305
    Wait-Condition { [NoticeProbe]::IsWindowVisible($mainWindow) -and [NoticeProbe]::Title($mainWindow) -like 'QuotaDock *7 *' } 'notice over hidden HUD'
    Send-Command 308
    Wait-Condition { [NoticeProbe]::Title($mainWindow) -like 'QuotaDock *5 *' } 'second island notice'
    Send-Command 308
    Wait-Condition { -not [NoticeProbe]::IsWindowVisible($mainWindow) } 'hidden state restoration'
    $restoredSize = Get-Size
    if ($baseSize[0] -ne $restoredSize[0] -or $baseSize[1] -ne $restoredSize[1]) { throw 'HUD geometry changed after notice' }
    Write-Output 'PASS: manual dismissal; hidden state and HUD geometry restored'

    Send-Command 100
    $script:expectedWidth = $null
    Send-Command 106
    Wait-Condition { $size = Get-Size; $size[0] -eq [Math]::Round(350 * [NoticeProbe]::GetDpiForWindow($mainWindow) / 96) -and $size[1] -eq [Math]::Round(218 * [NoticeProbe]::GetDpiForWindow($mainWindow) / 96) } 'full view'
    $fullSize = Get-Size
    $script:expectedWidth = $fullSize[0]
    Send-Command 305
    Wait-Condition { [NoticeProbe]::Title($mainWindow) -like 'QuotaDock *7 *' } 'notice over full view'
    Send-Command 308
    Wait-Condition { [NoticeProbe]::Title($mainWindow) -like 'QuotaDock *5 *' } 'second notice over full view'
    Send-Command 308
    Wait-Condition { $size = Get-Size; [NoticeProbe]::Title($mainWindow) -eq 'QuotaDock' -and $size[1] -eq $fullSize[1] } 'full view restoration'
    $restoredSize = Get-Size
    if ($fullSize[0] -ne $restoredSize[0] -or $fullSize[1] -ne $restoredSize[1]) { throw 'Full view geometry changed after notice' }
    $settings = Get-Content -LiteralPath (Join-Path $fixtureData 'widget-settings.json') -Raw | ConvertFrom-Json
    if ($settings.resetAlerts.style -ne 'island') { throw 'Island style was not persisted' }
    Write-Output 'PASS: full view restored; island selection persisted'
    Write-Output 'PASS: constant width throughout; opening has intermediate animation frames'
    Write-Output "PASS: original default size restored ($originalWidth px); $(@($heights | Select-Object -Unique).Count) sampled height steps"
} finally {
    if ($script:mainWindow -ne [IntPtr]::Zero) { Send-Command 199 }
    if (-not $process.WaitForExit(3000)) { $process.Kill() }
}
