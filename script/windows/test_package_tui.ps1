[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-FixtureFile {
    param(
        [string] $Path,
        [string] $Content
    )

    New-Item -ItemType Directory -Path (Split-Path $Path) -Force | Out-Null
    [System.IO.File]::WriteAllText($Path, $Content)
}

function Get-ZipEntry {
    param([string] $Path)

    Add-Type -AssemblyName System.IO.Compression
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Path)
    try {
        return @($archive.Entries.FullName | Sort-Object)
    } finally {
        $archive.Dispose()
    }
}

function Assert-SequenceEqual {
    param(
        [string[]] $Expected,
        [string[]] $Actual
    )

    if ([string]::Join("`n", $Expected) -cne [string]::Join("`n", $Actual)) {
        throw "Unexpected archive entries.`nExpected:`n$($Expected -join "`n")`nActual:`n$($Actual -join "`n")"
    }
}

$packageScript = Join-Path $PSScriptRoot 'package_tui.ps1'
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) "warp package tui $([guid]::NewGuid())"

try {
    $binaryPath = Join-Path $testRoot 'input/warp-tui-dev.exe'
    $pdbPath = Join-Path $testRoot 'input/warp_tui_dev.pdb'
    $resourcesDir = Join-Path $testRoot 'resources'
    $assetsDir = Join-Path $testRoot 'assets'
    Write-FixtureFile $binaryPath 'binary fixture'
    Write-FixtureFile $pdbPath 'symbols fixture'
    Write-FixtureFile (Join-Path $resourcesDir 'themes/dark.yaml') 'theme fixture'
    Write-FixtureFile (Join-Path $resourcesDir 'warp-logo.txt') 'logo fixture'
    Write-FixtureFile (Join-Path $assetsDir 'conpty.dll') 'conpty fixture'
    Write-FixtureFile (Join-Path $assetsDir 'OpenConsole.exe') 'console fixture'

    $first = & $packageScript `
        -Channel dev `
        -Architecture x64 `
        -BinaryPath $binaryPath `
        -PdbPath $pdbPath `
        -ResourcesDir $resourcesDir `
        -WindowsAssetsDir $assetsDir `
        -OutputDir (Join-Path $testRoot 'first')
    $second = & $packageScript `
        -Channel dev `
        -Architecture x64 `
        -BinaryPath $binaryPath `
        -PdbPath $pdbPath `
        -ResourcesDir $resourcesDir `
        -WindowsAssetsDir $assetsDir `
        -OutputDir (Join-Path $testRoot 'second')
    $arm64 = & $packageScript `
        -Channel stable `
        -Architecture arm64 `
        -BinaryPath $binaryPath `
        -PdbPath $pdbPath `
        -ResourcesDir $resourcesDir `
        -WindowsAssetsDir $assetsDir `
        -OutputDir (Join-Path $testRoot 'arm64')

    $expectedX64Entries = @(
        'conpty.dll',
        'resources/themes/dark.yaml',
        'resources/warp-logo.txt',
        'warp-tui-dev.exe',
        'x64/OpenConsole.exe'
    )
    Assert-SequenceEqual `
        -Expected $expectedX64Entries `
        -Actual (Get-ZipEntry $first.ArchivePath)
    Assert-SequenceEqual `
        -Expected @('warp-tui-dev.pdb') `
        -Actual (Get-ZipEntry $first.SymbolsArchivePath)
    if ([System.IO.Path]::GetFileName($arm64.ArchivePath) -cne 'warp-tui-stable-windows-aarch64.zip') {
        throw "Unexpected arm64 archive name: $($arm64.ArchivePath)"
    }
    $expectedArm64Entries = @(
        'arm64/OpenConsole.exe',
        'conpty.dll',
        'resources/themes/dark.yaml',
        'resources/warp-logo.txt',
        'warp-tui-stable.exe'
    )
    Assert-SequenceEqual `
        -Expected $expectedArm64Entries `
        -Actual (Get-ZipEntry $arm64.ArchivePath)

    $firstHash = (Get-FileHash -Algorithm SHA256 $first.ArchivePath).Hash
    $secondHash = (Get-FileHash -Algorithm SHA256 $second.ArchivePath).Hash
    if ($firstHash -cne $secondHash) {
        throw 'Application archives are not deterministic'
    }
    $firstSymbolsHash = (Get-FileHash -Algorithm SHA256 $first.SymbolsArchivePath).Hash
    $secondSymbolsHash = (Get-FileHash -Algorithm SHA256 $second.SymbolsArchivePath).Hash
    if ($firstSymbolsHash -cne $secondSymbolsHash) {
        throw 'Symbols archives are not deterministic'
    }

    Remove-Item -LiteralPath (Join-Path $assetsDir 'conpty.dll')
    $missingAssetFailed = $false
    try {
        & $packageScript `
            -Channel dev `
            -Architecture x64 `
            -BinaryPath $binaryPath `
            -PdbPath $pdbPath `
            -ResourcesDir $resourcesDir `
            -WindowsAssetsDir $assetsDir `
            -OutputDir (Join-Path $testRoot 'missing') | Out-Null
    } catch {
        $missingAssetFailed = $true
    }
    if (-not $missingAssetFailed) {
        throw 'Packaging succeeded without conpty.dll'
    }
} finally {
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}

Write-Output 'Windows TUI packaging tests passed'
