$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $projectRoot 'dist/QuotaDock.exe'
$fixtureRoot = Join-Path $projectRoot 'artifacts/desktop-notification-check'
$fixtureData = Join-Path $fixtureRoot 'cn.codex.quota.widget'
$instance=[Guid]::NewGuid().ToString(); $className="QuotaDockNativeWindow-$instance"
New-Item -ItemType Directory -Path $fixtureData -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $fixtureData 'widget-settings.json'), '{"version":1,"codexTaskSource":"both","compactScale":0.46,"compactScaleVersion":2,"resetAlerts":{"enabled":false,"style":"popup"}}',(New-Object Text.UTF8Encoding $false))
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
$codexFixture=Join-Path $fixtureRoot 'codex-home'
$sessionFolder=Join-Path $codexFixture 'sessions/2026/10/21'
New-Item -ItemType Directory -Force $sessionFolder | Out-Null
$sessionId=[Guid]::NewGuid().ToString(); $sessionFile=Join-Path $sessionFolder "rollout-$sessionId.jsonl"
$encoding=New-Object Text.UTF8Encoding $false
$header=@{type='session_meta';payload=@{id=$sessionId;cwd='D:\Code\桌面完成测试';source='vscode';originator='Codex Desktop'}} | ConvertTo-Json -Depth 6 -Compress
$old=@{type='event_msg';timestamp=[DateTime]::UtcNow.AddMinutes(-10).ToString('o');payload=@{type='task_complete';turn_id='old';last_agent_message='Old event must not replay'}} | ConvertTo-Json -Depth 6 -Compress
[IO.File]::WriteAllText($sessionFile,$header+"`n"+$old+"`n",$encoding)
$launch.EnvironmentVariables['CODEX_HOME']=$codexFixture
$launch.EnvironmentVariables['QUOTADOCK_TEST_INSTANCE']=$instance
$launch.FileName=$exe; $launch.UseShellExecute=$false; $launch.EnvironmentVariables['APPDATA']=$fixtureRoot
$process=[Diagnostics.Process]::Start($launch)
$script:session=[Guid]::NewGuid().ToString()
try {
    Wait-For { $script:window=[TaskProbe]::Find($process.Id,$className);$script:window -ne [IntPtr]::Zero } 'test window'
    $baseSize=Get-Size
    Start-Sleep -Milliseconds 500
    if ([TaskProbe]::Title($window) -ne 'QuotaDock') { throw 'Old completion replayed' }
    $started=@{type='event_msg';timestamp=[DateTime]::UtcNow.ToString('o');payload=@{type='task_started';turn_id='desktop-1'}} | ConvertTo-Json -Depth 6 -Compress
    [IO.File]::AppendAllText($sessionFile,$started+"`n",$encoding)
    Start-Sleep -Milliseconds 250
    if ([TaskProbe]::Title($window) -ne 'QuotaDock') { throw 'Task start caused notification' }
    $done=@{type='event_msg';timestamp=[DateTime]::UtcNow.ToString('o');payload=@{type='task_complete';turn_id='desktop-1';last_agent_message='诊断：桌面本轮完成，摘要为中文消息。'}} | ConvertTo-Json -Depth 6 -Compress
    $split=[int]($done.Length/2)
    [IO.File]::AppendAllText($sessionFile,$done.Substring(0,$split),$encoding)
    Start-Sleep -Milliseconds 250
    if ([TaskProbe]::Title($window) -ne 'QuotaDock') { throw 'Partial record caused notification' }
    [IO.File]::AppendAllText($sessionFile,$done.Substring($split)+"`n",$encoding)
    Wait-For { [TaskProbe]::Title($window) -like '*桌面完成测试*本轮结束' } 'desktop completion notification'
    Start-Sleep -Milliseconds 400
    if (-not [TaskProbe]::IsWindowVisible($window) -or (Get-Size)[0] -ne $baseSize[0]) { throw 'Desktop island not visible or width changed' }
    $receipt=Get-Content (Join-Path $fixtureData 'codex-hook-last.json') -Raw | ConvertFrom-Json
    if ($receipt.detail -ne 'localSession') { throw 'Incorrect delivery source' }
    $script:session=$sessionId
    Send-Hook 'desktop-1' 'Duplicate arriving through the Hook channel.'
    [IO.File]::AppendAllText($sessionFile,$done+"`n",$encoding)
    Start-Sleep -Milliseconds 1100
    Send-Command 308
    Wait-For { [TaskProbe]::Title($window) -eq 'QuotaDock' -and (Get-Size)[1] -eq $baseSize[1] } 'deduplicated desktop completion restoration'
    Write-Output 'PASS: native directory change -> desktop completion -> visible island -> HUD restoration'
    Write-Output 'PASS: no historical replay; no task-start or partial-record alerts; repeated completion and Hook/local cross-channel deduplication'
} finally {
    if ($script:window -ne [IntPtr]::Zero) { Send-Command 199 }
    if (-not $process.WaitForExit(3000)) { $process.Kill() }
}
