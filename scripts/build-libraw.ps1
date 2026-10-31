#Requires -Version 5.1
param(
    [ValidateSet('arm64', 'x64')]
    [string]$Arch = $(if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }),
    [ValidateSet('arm64', 'x64')]
    [string]$HostArch = $(if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' })
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $root '.cache/native-deps'
$archive = Join-Path $cache 'LibRaw-0.22.3.tar.gz'
$source = Join-Path $cache 'LibRaw-0.22.3-msvc'
$expected = '7edf96c5b70cff88980a02ab0fc8af125cb6a4a377985573c654439d9d02355c'

if (-not (Get-Command tar.exe -ErrorAction SilentlyContinue)) {
    throw 'tar.exe is required to extract the pinned LibRaw archive.'
}
New-Item -ItemType Directory -Force $cache | Out-Null

if (-not (Test-Path $archive)) {
    Invoke-WebRequest -Uri 'https://codeload.github.com/LibRaw/LibRaw/tar.gz/refs/tags/0.22.3' `
        -OutFile $archive -UseBasicParsing
}
$actual = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
if ($actual -ne $expected) {
    throw "LibRaw archive SHA-256 mismatch: got $actual, expected $expected"
}

if (-not (Test-Path (Join-Path $source 'Makefile.msvc'))) {
    New-Item -ItemType Directory -Force $source | Out-Null
    tar -xzf $archive -C $source --strip-components=1
    if ($LASTEXITCODE -ne 0) {
        throw "Could not extract $archive"
    }
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (-not (Test-Path $vswhere)) {
    throw 'Visual Studio Build Tools with vswhere.exe is required.'
}
$component = if ($Arch -eq 'arm64') {
    'Microsoft.VisualStudio.Component.VC.Tools.ARM64'
} else {
    'Microsoft.VisualStudio.Component.VC.Tools.x86.x64'
}
$installation = & $vswhere -latest -products * -requires $component -property installationPath |
    Select-Object -First 1
if (-not $installation) {
    $installation = & $vswhere -latest -products * -property installationPath | Select-Object -First 1
}
if (-not $installation) {
    throw 'No Visual Studio installation with C++ build tools was found.'
}
$devcmd = Join-Path $installation 'Common7/Tools/VsDevCmd.bat'
if (-not (Test-Path $devcmd)) {
    throw "VsDevCmd.bat not found under $installation"
}

Push-Location $source
try {
    $command = "call `"$devcmd`" -arch=$Arch -host_arch=$HostArch >nul && nmake /f Makefile.msvc COPT_OPT=/openmp lib\libraw_static.lib"
    & cmd.exe /d /c $command
    if ($LASTEXITCODE -ne 0) {
        throw "LibRaw build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$library = Join-Path $source 'lib/libraw_static.lib'
if (-not (Test-Path $library)) {
    throw "LibRaw did not produce $library"
}
Write-Host "LibRaw built: $library"
