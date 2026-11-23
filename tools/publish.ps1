# Build the release exe and make the distribution zip and its SHA256 (spec 9.1).
# ASCII only (PowerShell 5.1 reads BOM-less files as cp932).
# Usage: powershell -ExecutionPolicy Bypass -File tools\publish.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$toml = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($toml -notmatch '(?m)^version\s*=\s*"([^"]+)"') { throw 'version not found in Cargo.toml' }
$version = $Matches[1]

cargo build --release -p procsight-app
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$name = "procsight-v$version-windows-x64"
$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }

# regedit expects CRLF: normalize the .reg file whatever the working tree has.
$regText = [System.IO.File]::ReadAllText((Join-Path $root 'tools\restore-taskmgr.reg'))
$regText = ($regText -replace "`r`n", "`n") -replace "`n", "`r`n"
$regTmp = Join-Path $dist 'restore-taskmgr.reg'
[System.IO.File]::WriteAllText($regTmp, $regText, (New-Object System.Text.ASCIIEncoding))

# Files go to the zip root, so entry names never contain a path separator.
$files = @(
    @{ Src = 'target\release\ProcSight.exe'; Name = 'ProcSight.exe' },
    @{ Src = 'dist\restore-taskmgr.reg'; Name = 'restore-taskmgr.reg' },
    @{ Src = 'README.md'; Name = 'README.md' },
    @{ Src = 'README.en.md'; Name = 'README.en.md' },
    @{ Src = 'LICENSE'; Name = 'LICENSE' },
    @{ Src = 'THIRD-PARTY-NOTICES.md'; Name = 'THIRD-PARTY-NOTICES.md' },
    @{ Src = 'CHANGELOG.md'; Name = 'CHANGELOG.md' }
)

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$stream = [System.IO.File]::Open($zip, [System.IO.FileMode]::CreateNew)
$archive = New-Object System.IO.Compression.ZipArchive($stream, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($f in $files) {
        $src = Join-Path $root $f.Src
        if (-not (Test-Path $src)) { throw "missing: $($f.Src)" }
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $src, $f.Name) | Out-Null
    }
} finally {
    $archive.Dispose()
    $stream.Dispose()
    Remove-Item $regTmp -Force
}

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
$sumFile = "$zip.sha256"
[System.IO.File]::WriteAllText($sumFile, "$hash  $name.zip`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Output "zip: $zip"
Write-Output "sha256: $hash"
