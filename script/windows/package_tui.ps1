[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('local', 'dev', 'preview', 'stable', 'oss')]
    [string] $Channel,
    [Parameter(Mandatory)]
    [ValidateSet('x64', 'arm64')]
    [string] $Architecture,
    [Parameter(Mandatory)]
    [string] $BinaryPath,
    [Parameter(Mandatory)]
    [string] $PdbPath,
    [Parameter(Mandatory)]
    [string] $ResourcesDir,
    [Parameter(Mandatory)]
    [string] $WindowsAssetsDir,
    [Parameter(Mandatory)]
    [string] $OutputDir,
    [switch] $RequireSignatures
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-File {
    param([string] $Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Required file does not exist: $Path"
    }
}

function Assert-ValidSignature {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute(
        'PSUseCompatibleCommands',
        '',
        Justification = 'This packaging script only runs on Windows.'
    )]
    param([string] $Path)

    if (-not (Get-Command Get-AuthenticodeSignature -ErrorAction SilentlyContinue)) {
        throw 'Authenticode signature validation is unavailable'
    }

    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ("$($signature.Status)" -ne 'Valid') {
        throw "File does not have a valid Authenticode signature: $Path ($($signature.Status))"
    }
}

function Add-ArchiveFile {
    param(
        [hashtable] $Files,
        [string] $EntryName,
        [string] $SourcePath
    )

    if ($Files.ContainsKey($EntryName)) {
        throw "Duplicate archive entry: $EntryName"
    }
    $Files[$EntryName] = $SourcePath
}

function Write-DeterministicZip {
    param(
        [hashtable] $Files,
        [string] $Path
    )

    Add-Type -AssemblyName System.IO.Compression
    $fileStream = [System.IO.File]::Open(
        $Path,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::ReadWrite,
        [System.IO.FileShare]::None
    )
    try {
        $archive = [System.IO.Compression.ZipArchive]::new(
            $fileStream,
            [System.IO.Compression.ZipArchiveMode]::Create,
            $false
        )
        try {
            $timestamp = [System.DateTimeOffset]::new(
                1980,
                1,
                1,
                0,
                0,
                0,
                [System.TimeSpan]::Zero
            )
            $entryNames = [string[]] $Files.Keys
            [System.Array]::Sort($entryNames, [System.StringComparer]::Ordinal)
            foreach ($entryName in $entryNames) {
                $entry = $archive.CreateEntry(
                    $entryName,
                    [System.IO.Compression.CompressionLevel]::Optimal
                )
                $entry.LastWriteTime = $timestamp
                $entryStream = $entry.Open()
                try {
                    $sourceStream = [System.IO.File]::OpenRead($Files[$entryName])
                    try {
                        $sourceStream.CopyTo($entryStream)
                    } finally {
                        $sourceStream.Dispose()
                    }
                } finally {
                    $entryStream.Dispose()
                }
            }
        } finally {
            $archive.Dispose()
        }
    } finally {
        $fileStream.Dispose()
    }
}

$binaryName = switch ($Channel) {
    'local' { 'warp-tui.exe' }
    default { "warp-tui-$Channel.exe" }
}
$publicArchitecture = switch ($Architecture) {
    'x64' { 'x86_64' }
    'arm64' { 'aarch64' }
}
$conptyPath = Join-Path $WindowsAssetsDir 'conpty.dll'
$openConsolePath = Join-Path $WindowsAssetsDir 'OpenConsole.exe'

Assert-File $BinaryPath
Assert-File $PdbPath
Assert-File $conptyPath
Assert-File $openConsolePath
if (-not (Test-Path -LiteralPath $ResourcesDir -PathType Container)) {
    throw "Required resources directory does not exist: $ResourcesDir"
}

if ($RequireSignatures) {
    Assert-ValidSignature $BinaryPath
    Assert-ValidSignature $conptyPath
    Assert-ValidSignature $openConsolePath
}

$archiveFiles = @{}
Add-ArchiveFile -Files $archiveFiles -EntryName $binaryName -SourcePath $BinaryPath
Add-ArchiveFile -Files $archiveFiles -EntryName 'conpty.dll' -SourcePath $conptyPath
Add-ArchiveFile `
    -Files $archiveFiles `
    -EntryName "$Architecture/OpenConsole.exe" `
    -SourcePath $openConsolePath

$resourceFiles = @(Get-ChildItem -LiteralPath $ResourcesDir -File -Recurse)
if ($resourceFiles.Count -eq 0) {
    throw "Resources directory is empty: $ResourcesDir"
}
$resourcesRoot = [System.IO.Path]::GetFullPath($ResourcesDir)
$directorySeparator = [System.IO.Path]::DirectorySeparatorChar.ToString()
if (-not $resourcesRoot.EndsWith($directorySeparator)) {
    $resourcesRoot += $directorySeparator
}
foreach ($resourceFile in $resourceFiles) {
    $resourcePath = [System.IO.Path]::GetFullPath($resourceFile.FullName)
    $relativePath = $resourcePath.Substring($resourcesRoot.Length).Replace('\', '/')
    Add-ArchiveFile `
        -Files $archiveFiles `
        -EntryName "resources/$relativePath" `
        -SourcePath $resourceFile.FullName
}

New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
$archiveBaseName = "warp-tui-$Channel-windows-$publicArchitecture"
$archivePath = Join-Path $OutputDir "$archiveBaseName.zip"
$symbolsArchivePath = Join-Path $OutputDir "$archiveBaseName-symbols.zip"
Write-DeterministicZip -Files $archiveFiles -Path $archivePath
$pdbName = [System.IO.Path]::ChangeExtension($binaryName, '.pdb')
Write-DeterministicZip -Files @{ $pdbName = $PdbPath } -Path $symbolsArchivePath

[PSCustomObject]@{
    ArchivePath = $archivePath
    SymbolsArchivePath = $symbolsArchivePath
}
