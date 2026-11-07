$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $projectRoot 'dist/QuotaDock.exe'
$fixtureRoot = Join-Path $projectRoot 'artifacts/task-notification-check'
$fixtureData = Join-Path $fixtureRoot 'cn.codex.quota.widget'
$instance=[Guid]::NewGuid().ToString(); $className="QuotaDockNativeWindow-$instance"
New-Item -ItemType Directory -Path $fixtureData -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $fixtureData 'widget-settings.json'), '{"version":1,"codexTaskSource":"hook","compactScale":0.46,"compactScaleVersion":2,"resetAlerts":{"enabled":false,"style":"popup"}}',(New-Object Text.UTF8Encoding $false))
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class TaskProbe {
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
[TaskProbe]::SetThreadDpiAwarenessContext([IntPtr](-4)) | Out-Null


function Wait-For([scriptblock]$Condition, [string]$Label) {
    $deadline=(Get-Date).AddSeconds(6)
    do { if (& $Condition) { return }; Start-Sleep -Milliseconds 50 } while ((Get-Date) -lt $deadline)
    throw "Timeout: $Label"
}
function Send-Command([int]$Id) { [TaskProbe]::PostMessage($script:window,0x111,[IntPtr]$Id,[IntPtr]::Zero) | Out-Null }
function Get-Size {
    $r=New-Object TaskProbe+Rect
    [TaskProbe]::GetWindowRect($script:window,[ref]$r) | Out-Null
    return @(($r.Right-$r.Left),($r.Bottom-$r.Top))
}
function Send-Hook([string]$Turn,[string]$Message) {
    $child=New-Object Diagnostics.ProcessStartInfo
    $child.FileName=$exe; $child.Arguments='--codex-stop'; $child.UseShellExecute=$false
    $child.RedirectStandardInput=$true; $child.RedirectStandardOutput=$true; $child.RedirectStandardError=$true
    $child.EnvironmentVariables['QUOTADOCK_TEST_INSTANCE']=$instance
    $child.CreateNoWindow=$true; $child.EnvironmentVariables['APPDATA']=$fixtureRoot
    $helper=[Diagnostics.Process]::Start($child)
    $payload=@{hook_event_name='Stop';cwd='D:\Code\通知测试项目';session_id=$script:session;turn_id=$Turn;last_assistant_message=$Message} | ConvertTo-Json -Compress
    $bytes=[Text.Encoding]::UTF8.GetBytes($payload); $helper.StandardInput.BaseStream.Write($bytes,0,$bytes.Length); $helper.StandardInput.BaseStream.Flush()
    # Keep stdin open: a complete JSON object must be sufficient to finish.
    if (-not $helper.WaitForExit(2500)) { $helper.StandardInput.Close(); if (-not $helper.WaitForExit(500)) { $helper.Kill() }; throw "Hook waited for EOF after receiving complete JSON" }
    $helper.StandardInput.Close()
    $stdout=$helper.StandardOutput.ReadToEnd();$stderr=$helper.StandardError.ReadToEnd()
    if (-not $helper.WaitForExit(4000)) { $helper.Kill();throw 'Hook timed out' }
    if ($helper.ExitCode -ne 0 -or $stdout.Trim() -ne '{}' -or $stderr.Trim().Length -gt 0) { throw "Hook result: $stdout; $stderr" }
}
$launch=New-Object Diagnostics.ProcessStartInfo
$launch.EnvironmentVariables['QUOTADOCK_TEST_INSTANCE']=$instance
$launch.FileName=$exe; $launch.UseShellExecute=$false; $launch.EnvironmentVariables['APPDATA']=$fixtureRoot
$process=[Diagnostics.Process]::Start($launch)
$script:session=[Guid]::NewGuid().ToString()
try {
    Wait-For { $script:window=[TaskProbe]::Find($process.Id,$className);$script:window -ne [IntPtr]::Zero } 'test window'
    $baseSize=Get-Size
    Send-Hook 'round-1' '诊断通知：本轮已完成。这是 Hook 到灵动岛的中文链路测试。'
    Wait-For { [TaskProbe]::Title($window) -like '*通知测试项目*本轮结束' } 'project name in notification'
    Start-Sleep -Milliseconds 500
    $size=Get-Size
    if ($size[0] -ne $baseSize[0] -or $size[1] -le $baseSize[1]) { throw 'Island geometry is incorrect' }
    if (-not [TaskProbe]::IsWindowVisible($window)) { throw 'Island is not visible' }
    if ([TaskProbe]::Find($process.Id,'#32770') -ne [IntPtr]::Zero) { throw 'Task notification used a popup' }
    Send-Hook 'round-1' '诊断通知：重复轮次不得再次入队。'
    Start-Sleep -Milliseconds 1200
    Send-Command 308
    Wait-For { [TaskProbe]::Title($window) -eq 'QuotaDock' -and (Get-Size)[1] -eq $baseSize[1] } 'deduplicated notice restores HUD'
    Write-Output 'PASS: UTF-8 Hook stdin; project name; visible island; fixed width; deduplication; restore'
    Write-Output 'PASS: task island independent of reset master and popup style'
    Send-Command 320
    Start-Sleep -Milliseconds 150
    Send-Hook 'round-off' 'Disabled notification must not show.'
    Start-Sleep -Milliseconds 1300
    if ([TaskProbe]::Title($window) -ne 'QuotaDock') { throw 'Disabled task notification displayed' }
    $settings=Get-Content (Join-Path $fixtureData 'widget-settings.json') -Raw | ConvertFrom-Json
    if ($settings.codexTaskNotifications -ne $false) { throw 'Task preference not persisted' }
    Send-Command 320
    Send-Command 100
    Start-Sleep -Milliseconds 150
    Send-Hook 'round-hidden' '诊断通知：隐藏的窗口也应显示消息，结束后恢复隐藏。'
    Wait-For { [TaskProbe]::IsWindowVisible($window) -and [TaskProbe]::Title($window) -like '*本轮结束' } 'notice over hidden HUD'
    Send-Command 308
    Wait-For { -not [TaskProbe]::IsWindowVisible($window) } 'hidden HUD restoration'
    Write-Output 'PASS: independent toggle; persisted setting; notification over hidden HUD; hidden state restoration'
} finally {
    if ($script:window -ne [IntPtr]::Zero) { Send-Command 199 }
    if (-not $process.WaitForExit(3000)) { $process.Kill() }
}
