#Requires -Version 5.1
# Prepare the Windows toolchain this checkout expects: portable CMake, Ninja and the DirectX
# Shader Compiler under .cache/tools, then LibRaw under .cache/native-deps.
#
# CMake and Ninja come from the vendor releases rather than a system install so that native
# builds behave the same on every machine; build.rs finds them here when they are absent from
# PATH. DXC supplies the DX12 shader compiler next to the executable for the DX12 backend.
param(
    [switch]$SkipLibRaw,
    [switch]$SkipDxc
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$tools = Join-Path $root '.cache/tools'
$downloads = Join-Path $tools 'zips'
New-Item -ItemType Directory -Force $downloads | Out-Null

function Get-PinnedArchive {
    param(
        [string]$Name,
        [string]$Url,
        [string]$Sha256
    )
    $archive = Join-Path $downloads $Name
    New-Item -ItemType Directory -Force (Split-Path -Parent $archive) | Out-Null
    if (-not (Test-Path $archive)) {
        Write-Host "Downloading $Name"
        Invoke-WebRequest -Uri $Url -OutFile $archive -UseBasicParsing
    }
    $actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
    if ($actual -ne $Sha256) {
        throw "$Name SHA-256 mismatch: got $actual, expected $Sha256"
    }
    return $archive
}

function Expand-ArchiveChecked {
    param(
        [string]$Archive,
        [string]$Destination,
        [string]$Marker
    )
    if (Test-Path (Join-Path $Destination $Marker)) {
        Write-Host "Already present: $Destination"
        return
    }
    New-Item -ItemType Directory -Force $Destination | Out-Null
    Expand-Archive -LiteralPath $Archive -DestinationPath $Destination -Force
    if (-not (Test-Path (Join-Path $Destination $Marker))) {
        throw "Archive $Archive did not contain $Marker"
    }
    Write-Host "Extracted to $Destination"
}

# CMake: the archive holds a single cmake-<version>-windows-x86_64 directory.
$cmakeArchive = Get-PinnedArchive -Name 'cmake-4.4.4-windows-x86_64.zip' `
    -Url 'https://github.com/Kitware/CMake/releases/download/v4.4.4/cmake-4.4.4-windows-x86_64.zip' `
    -Sha256 'bace36e94b31c68ab6fa295f26dfa11219e0701cf7c94b0284a7d1cb13dac536'
$cmakeRoot = Join-Path $tools 'cmake'
Expand-ArchiveChecked -Archive $cmakeArchive -Destination $cmakeRoot -Marker 'cmake-4.4.4-windows-x86_64/bin/cmake.exe'

# Ninja: the archive is a flat ninja.exe.
$ninjaArchive = Get-PinnedArchive -Name 'ninja-win.zip' `
    -Url 'https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-win.zip' `
    -Sha256 '07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65'
Expand-ArchiveChecked -Archive $ninjaArchive -Destination (Join-Path $tools 'ninja') -Marker 'ninja.exe'

if (-not $SkipDxc) {
    # DXC: only bin/x64 is used, but the archive is kept whole so the layout stays plain.
    $dxcArchive = Get-PinnedArchive -Name 'dxc_2026_09_29.zip' `
        -Url 'https://github.com/microsoft/DirectXShaderCompiler/releases/download/v1.9.2609/dxc_2026_09_29.zip' `
        -Sha256 'ad31b1fc8443175d204f77a611fdb3ef2ec42759bdc2f1167368de24a4a7e7f1'
    Expand-ArchiveChecked -Archive $dxcArchive -Destination (Join-Path $tools 'dxc') -Marker 'bin/x64/dxcompiler.dll'

    # The DX12 backend loads dxcompiler.dll beside the executable.
    $binary = Join-Path $root 'target/release'
    if (Test-Path $binary) {
        Copy-Item (Join-Path $tools 'dxc/bin/x64/dxcompiler.dll') $binary -Force
        Copy-Item (Join-Path $tools 'dxc/bin/x64/dxil.dll') $binary -Force
    }
}

if (-not $SkipLibRaw) {
    & (Join-Path $PSScriptRoot 'build-libraw.ps1') -Arch x64 -HostArch x64
}

Write-Host 'Windows toolchain ready. Build with: cargo build-app'
