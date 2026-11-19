#Requires -Version 5.1
param([ValidateSet('arm64', 'x64')][string]$Arch = 'x64',
      [ValidateSet('arm64', 'x64')][string]$HostArch = 'x64')
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
. (Join-Path $PSScriptRoot 'windows-native.ps1')
$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $root '.cache/native-deps'
$pins = Get-Content -Raw (Join-Path $root 'native/openmp/pins.txt') | ConvertFrom-StringData
$version = $pins.version
$prefix = Join-Path $cache "openmp-$version-windows-$Arch-install"
if ((Test-Path "$prefix/include/omp.h") -and (Test-Path "$prefix/lib/libomp.lib") -and
    (Test-Path "$prefix/bin/libomp.dll")) { return }
$tools = Get-NativeWindowsTools -Root $root -Arch $Arch -HostArch $HostArch
$source = Join-Path $cache "llvm-project-$version.src"
$member = "llvm-project-$version.src"
Get-PinnedNativeSource -Archive (Join-Path $cache "$member.tar.xz") `
    -Url "https://github.com/llvm/llvm-project/releases/download/llvmorg-$version/$member.tar.xz" `
    -Sha256 $pins.sha256 -Destination $source -Marker 'openmp/CMakeLists.txt' `
    -Members @("$member/openmp", "$member/cmake", "$member/llvm/cmake", "$member/LICENSE.TXT")
$build = Join-Path $cache "openmp-$version-windows-$Arch-build"
$clang = $tools.Clang
$c = Join-Path (Split-Path -Parent $clang) 'clang.exe'
$configure = @(
    "`"$($tools.CMake)`"", "-S `"$source/openmp`"", "-B `"$build`"", '-G Ninja',
    '-DCMAKE_BUILD_TYPE=Release', "-DCMAKE_MAKE_PROGRAM=`"$($tools.Ninja)`"",
    "-DCMAKE_C_COMPILER=`"$c`"",
    "-DCMAKE_CXX_COMPILER=`"$clang`"",
    "-DCMAKE_C_COMPILER_TARGET=$($tools.Target)",
    "-DCMAKE_CXX_COMPILER_TARGET=$($tools.Target)",
    "-DCMAKE_INSTALL_PREFIX=`"$prefix`"",
    '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL',
    '-DLIBOMP_ENABLE_SHARED=ON', '-DLIBOMP_INSTALL_ALIASES=OFF',
    '-DOPENMP_ENABLE_LIBOMPTARGET=OFF', '-DOPENMP_MSVC_NAME_SCHEME=OFF'
) -join ' '
Invoke-NativeWindowsCommand $tools "$configure && `"$($tools.CMake)`" --build `"$build`" --parallel 4 && `"$($tools.CMake)`" --install `"$build`""
if (-not (Test-Path "$prefix/bin/libomp.dll") -or -not (Test-Path "$prefix/lib/libomp.lib")) {
    throw 'LLVM OpenMP did not produce the expected DLL and import library.'
}
New-Item -ItemType Directory -Force "$prefix/share/licenses" | Out-Null
Copy-Item "$source/openmp/LICENSE.TXT" "$prefix/share/licenses/LLVM-OpenMP.txt" -Force
