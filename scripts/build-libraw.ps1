#Requires -Version 5.1
param(
    [ValidateSet('arm64', 'x64')]
    [string]$Arch = $(if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }),
    [ValidateSet('arm64', 'x64')]
    [string]$HostArch = $(if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' })
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

. (Join-Path $PSScriptRoot 'windows-native.ps1')
$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $root '.cache/native-deps'
$archive = Join-Path $cache 'LibRaw-0.22.3.tar.gz'
$hashes = Get-Content -Raw (Join-Path $root 'native/libraw/dht-hashes.txt') | ConvertFrom-StringData
$directory = "LibRaw-0.22.3-$($hashes.patched.Substring(0, 12))-rawspeed-llvm-$Arch"
$source = Join-Path $cache $directory
$expected = '7edf96c5b70cff88980a02ab0fc8af125cb6a4a377985573c654439d9d02355c'

Get-PinnedNativeSource -Archive $archive `
    -Url 'https://codeload.github.com/LibRaw/LibRaw/tar.gz/refs/tags/0.22.3' `
    -Sha256 $expected -Destination $source -Marker 'Makefile.msvc'

# Both platforms apply the same correction and verify the complete resulting file.
$dht = Join-Path $source 'src/demosaic/dht_demosaic.cpp'
$actual = (Get-FileHash -Algorithm SHA256 $dht).Hash.ToLowerInvariant()
if ($actual -eq $hashes.upstream) {
    & git -c core.autocrlf=false -c core.eol=lf -C $root apply --directory=".cache/native-deps/$directory" native/libraw/dht.patch
    if ($LASTEXITCODE -ne 0) { throw 'Could not apply the pinned LibRaw DHT correction.' }
} elseif ($actual -ne $hashes.patched) {
    throw 'LibRaw DHT source differs from the pinned upstream and patched revisions.'
}
if ((Get-FileHash -Algorithm SHA256 $dht).Hash.ToLowerInvariant() -ne $hashes.patched) {
    throw 'Patched LibRaw DHT source SHA-256 mismatch.'
}

$tools = Get-NativeWindowsTools -Root $root -Arch $Arch -HostArch $HostArch
& (Join-Path $PSScriptRoot 'build-rawspeed.ps1') -Arch $Arch -HostArch $HostArch
$pins = Get-Content -Raw (Join-Path $root 'native/rawspeed/pins.txt') | ConvertFrom-StringData
$omp = Get-Content -Raw (Join-Path $root 'native/openmp/pins.txt') | ConvertFrom-StringData
$openmp = Join-Path $cache "openmp-$($omp.version)-windows-$Arch-install"
$rawspeed = Join-Path $cache "rawspeed-$($pins.revision.Substring(0,12))-windows-$Arch"
$clangBin = Split-Path -Parent $tools.Clang
Push-Location $source
try {
    $environment = @(
        "set `"PATH=$openmp/bin;$clangBin;%PATH%`"",
        "set `"INCLUDE=$openmp/include;$root/native/rawspeed;$rawspeed/include;$rawspeed/include/rawspeed;%INCLUDE%`"",
        "set `"LIB=$openmp/lib;$rawspeed/lib;%LIB%`""
    ) -join ' && '
    $target = "--target=$($tools.Target)"
    $build = "nmake /f Makefile.msvc CC=clang-cl `"COPT_OPT=/clang:-fopenmp $target /DUSE_RAWSPEED3`" lib\libraw_static.lib"
    $adapter = "clang-cl /nologo /std:c++20 /EHsc /MD /O2 /clang:-fopenmp $target /c `"$root/native/rawspeed/adapter.cpp`" /Foobject\rawspeed_adapter.obj"
    Invoke-NativeWindowsCommand $tools "$environment && $build && $adapter"
    foreach ($check in @('dht_safety', 'rawspeed_adoption')) {
        $testSource = if ($check -eq 'dht_safety') { "$root/native/libraw/dht_safety.cpp" } else { "$root/native/rawspeed/adoption_tests.cpp" }
        $test = @(
            "clang-cl /nologo /std:c++17 /EHsc /MD /O1 /clang:-fopenmp $target",
            "/DWIN32 /DLIBRAW_NODLL /I. `"$testSource`" object\rawspeed_adapter.obj",
            "lib\libraw_static.lib rawspeed.lib pugixml.lib libomp.lib",
            "/Foobject\$check.obj /Felib\$check.exe"
        ) -join ' '
        Invoke-NativeWindowsCommand $tools "$environment && $test && lib\$check.exe `"$root/tests/fixtures/raw/patches.dng`""
    }
} finally {
    Pop-Location
}
$library = Join-Path $source 'lib/libraw_static.lib'
if (-not (Test-Path $library)) { throw "LibRaw did not produce $library" }
Set-Content -Encoding Ascii -Path (Join-Path $source 'antitone-features.txt') -Value "rawspeed3;llvm-openmp-$($omp.version)"
Write-Host "LibRaw built: $library"
