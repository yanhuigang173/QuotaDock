param([string]$Version = '0.4.0')

$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+([-.][A-Za-z0-9.-]+)?$') {
    throw 'Version must be a version number such as 0.4.0.'
}
$projectRoot = Split-Path -Parent $PSScriptRoot
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$outputRoot = Join-Path $projectRoot 'out/github'
$folderName = "QuotaDock-$Version-source-$stamp"
$sourceRoot = Join-Path $outputRoot $folderName
if (Test-Path -LiteralPath $sourceRoot) { throw "Output already exists: $sourceRoot" }
New-Item -ItemType Directory -Path $sourceRoot -Force | Out-Null

# Explicit source directories prevent build caches, local Codex configuration,
# diagnostic records and release executables from entering the upload package.
$directories = @(
    'src-native/src', 'src-native/examples', 'src-tauri/src',
    'src-tauri/capabilities', 'src/main', 'src/shared', 'src/renderer',
    'src/settings', 'scripts', 'docs', '.github'
)
$files = @(
    '.gitignore', '.gitattributes', 'README.md', 'LICENSE',
    'THIRD_PARTY_NOTICES.md', 'CONTRIBUTING.md', 'CHANGELOG.md', 'package.json',
    'src-native/Cargo.toml', 'src-native/Cargo.lock', 'src-native/build.rs',
    'src-native/resource.rc', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock',
    'src-tauri/build.rs', 'src-tauri/tauri.conf.json'
)
foreach ($relative in $directories) {
    $directory = Join-Path $projectRoot $relative
    foreach ($file in Get-ChildItem -LiteralPath $directory -Recurse -File) {
        $files += $file.FullName.Substring($projectRoot.Length + 1)
    }
}
foreach ($file in Get-ChildItem -LiteralPath (Join-Path $projectRoot 'assets') -File) {
    if ($file.Name -notmatch '^[1-5]\.png$') {
        $files += "assets/$($file.Name)"
    }
}
foreach ($relative in $files | Sort-Object -Unique) {
    $destination = Join-Path $sourceRoot $relative
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot $relative) -Destination $destination
}
$archive = Join-Path $outputRoot "$folderName.zip"
# Use .NET directly: Compress-Archive can omit dotfiles such as .gitignore.
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($sourceRoot, $archive)
Write-Output "Source folder: $sourceRoot"
Write-Output "Source archive: $archive"
Write-Output "Files: $((Get-ChildItem -LiteralPath $sourceRoot -Recurse -File -Force).Count)"
