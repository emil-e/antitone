#Requires -Version 5.1
param([ValidateSet('arm64', 'x64')][string]$Arch = 'x64',
      [ValidateSet('arm64', 'x64')][string]$HostArch = 'x64')
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'windows-native.ps1')
$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $root '.cache/native-deps'
$pins = Get-Content -Raw (Join-Path $root 'native/rawspeed/pins.txt') | ConvertFrom-StringData
$omp = Get-Content -Raw (Join-Path $root 'native/openmp/pins.txt') | ConvertFrom-StringData
& (Join-Path $PSScriptRoot 'build-openmp.ps1') -Arch $Arch -HostArch $HostArch
$openmp = Join-Path $cache "openmp-$($omp.version)-windows-$Arch-install"
$tools = Get-NativeWindowsTools -Root $root -Arch $Arch -HostArch $HostArch
$revision = $pins.revision
$source = Join-Path $cache "rawspeed-$revision"
$pugi = Join-Path $cache "pugixml-$($pins.pugixml_version)"
$prefix = Join-Path $cache "rawspeed-$($revision.Substring(0,12))-windows-$Arch"
Get-PinnedNativeSource -Archive (Join-Path $cache "rawspeed-$revision.tar.gz") `
    -Url "https://codeload.github.com/darktable-org/rawspeed/tar.gz/$revision" -Sha256 $pins.sha256 `
    -Destination $source -Marker 'CMakeLists.txt'
Get-PinnedNativeSource -Archive (Join-Path $cache "pugixml-$($pins.pugixml_version).tar.gz") `
    -Url "https://codeload.github.com/zeux/pugixml/tar.gz/refs/tags/v$($pins.pugixml_version)" `
    -Sha256 $pins.pugixml_sha256 -Destination $pugi -Marker 'CMakeLists.txt'
$build = Join-Path $prefix 'build'
$configure = @(
    "`"$($tools.CMake)`"", "-S `"$root/native/rawspeed`"", "-B `"$build`"", '-G Ninja',
    '-DCMAKE_BUILD_TYPE=Release', "-DCMAKE_MAKE_PROGRAM=`"$($tools.Ninja)`"",
    "-DCMAKE_CXX_COMPILER=`"$($tools.Clang)`"",
    "-DCMAKE_CXX_COMPILER_TARGET=$($tools.Target)",
    "-DCMAKE_INSTALL_PREFIX=`"$prefix`"",
    "-DRAWSPEED_SOURCE=`"$source`"", "-DPUGIXML_SOURCE=`"$pugi`"",
    "-DRAWSPEED_REVISION=$revision",
    '-DOpenMP_CXX_FLAGS=-fopenmp', '-DOpenMP_CXX_LIB_NAMES=omp',
    "-DOpenMP_omp_LIBRARY=`"$openmp/lib/libomp.lib`"",
    "-DOpenMP_CXX_INCLUDE_DIR=`"$openmp/include`""
) -join ' '
$test = "`"$build/adapter_tests.exe`" `"$root/tests/fixtures/raw/patches.dng`""
Invoke-NativeWindowsCommand $tools "set `"PATH=$openmp/bin;%PATH%`" && $configure && `"$($tools.CMake)`" --build `"$build`" --parallel 4 && `"$($tools.CMake)`" --install `"$build`" && $test"
