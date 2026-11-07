param([switch]$Offline)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$manifestPath = Join-Path $projectRoot 'src-native/Cargo.toml'

$buildArgs = @('build', '--manifest-path', $manifestPath, '--release', '--locked')
if ($Offline) { $buildArgs += '--offline' }
& cargo @buildArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$binaryPath = Join-Path $projectRoot 'src-native/target/release/quotadock-native.exe'
$outputDir = Join-Path $projectRoot 'dist'
New-Item -ItemType Directory -Path $outputDir -Force | Out-Null
$outputBinary = Join-Path $outputDir 'QuotaDock.exe'
$running = @(Get-Process -Name QuotaDock -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $outputBinary })
if ($running.Count -gt 0) {
    $running | Stop-Process -Force
    Start-Sleep -Milliseconds 300
}
for ($attempt = 1; $attempt -le 5; $attempt++) {
    try {
        Copy-Item -LiteralPath $binaryPath -Destination $outputBinary -Force
        break
    } catch {
        if ($attempt -eq 5) { throw }
        Start-Sleep -Milliseconds 300
    }
}

# Old WebView2 artifacts are not part of the native Windows build.
foreach ($oldName in @('WebView2Loader.dll', 'QuotaDock-portable.zip', 'CodexQuota.exe')) {
    $oldPath = Join-Path $outputDir $oldName
    if (Test-Path -LiteralPath $oldPath) { Remove-Item -LiteralPath $oldPath -Force }
}
Write-Output $outputBinary
