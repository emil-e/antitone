#Requires -Version 5.1
# Produce a portable Windows ZIP: the editor, the Visual C++ runtime it needs app-locally,
# and the notices that travel with the native and Rust components.
#
# The result is a local test artifact, not a declaration of release clearance; NOTICE.md owns
# what the packaged notices mean and what remains unresolved.
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root '.cache/dist'
$stage = Join-Path $dist 'Antitone-windows-x64'
$archive = Join-Path $dist 'Antitone-windows-x64.zip'

function Assert-PortableApplication {
    param([string]$Directory, [string]$Version)
    Push-Location -LiteralPath $Directory
    $originalPath = $env:PATH
    $originalEncoding = [Console]::OutputEncoding
    try {
        $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
        [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
        $smoke = @(& '.\antitone.exe' --version)
        if ($LASTEXITCODE -ne 0 -or $smoke.Count -ne 1 -or $smoke[0] -ne $Version) {
            throw "Packaged executable failed version check: '$smoke' (exit $LASTEXITCODE)"
        }
        $loaded = @(& '.\antitone.exe' --check-semantic-wb-runtime)
        if ($LASTEXITCODE -ne 0 -or $loaded.Count -ne 1) {
            throw "Packaged ONNX Runtime smoke failed: '$loaded' (exit $LASTEXITCODE)"
        }
        # Rust canonicalizes to an extended-length Windows path.
        $path = $loaded[0]
        if ($path.StartsWith('\\?\UNC\')) {
            $path = '\\' + $path.Substring(8)
        } elseif ($path.StartsWith('\\?\')) {
            $path = $path.Substring(4)
        }
        $expected = (Get-Item -LiteralPath 'onnxruntime.dll').FullName
        if ($path -ne $expected) { throw "Smoke check did not use packaged ONNX Runtime: '$path'" }
    } finally {
        [Console]::OutputEncoding = $originalEncoding
        $env:PATH = $originalPath
        Pop-Location
    }
}

if (-not $SkipBuild) {
    Push-Location $root
    try {
        & cargo build --locked --release --features editor --bin antitone
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

$binary = Join-Path $root 'target/release/antitone.exe'
if (-not (Test-Path $binary)) { throw "Missing $binary; run without -SkipBuild first." }
$version = (& $binary --version).Trim()

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item $binary $stage

# App-local deployment of the runtime the binary imports; no installer is required.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (-not (Test-Path $vswhere)) { throw 'vswhere.exe is required to locate the Visual C++ runtime.' }
$installation = & $vswhere -latest -products * -property installationPath | Select-Object -First 1
if (-not $installation) { throw 'No Visual Studio installation found for the runtime files.' }
. (Join-Path $PSScriptRoot 'windows-runtime.ps1')
$redistributables = @(Get-VcRuntimeFiles -RedistRoot (Join-Path $installation 'VC/Redist/MSVC'))
Copy-Item -LiteralPath $redistributables -Destination $stage

$omp = Get-Content -Raw (Join-Path $root 'native/openmp/pins.txt') | ConvertFrom-StringData
$openmp = Join-Path $root ".cache/native-deps/openmp-$($omp.version)-windows-x64-install"
Copy-Item -LiteralPath (Get-LlvmOpenMPRuntime -Prefix $openmp) -Destination $stage
$devcmd = Join-Path $installation 'Common7/Tools/VsDevCmd.bat'
$imports = & cmd.exe /d /c "call `"$devcmd`" -arch=x64 -host_arch=x64 >nul && dumpbin /dependents `"$binary`""
if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the production OpenMP imports.' }
Assert-LlvmOpenMPImports -Lines $imports

$onnxPinsPath = Join-Path $root 'native/onnxruntime/pins.txt'
$onnx = Get-Content -Raw $onnxPinsPath | ConvertFrom-StringData
$onnxPrefix = Join-Path $root ".cache/native-deps/onnxruntime-$($onnx.version)"
$onnxDll = Join-Path $onnxPrefix 'onnxruntime.dll'
if (-not (Test-Path -LiteralPath $onnxDll -PathType Leaf)) {
    throw "Missing $onnxDll; run scripts/setup-windows.ps1 first."
}
Copy-Item -LiteralPath $onnxDll -Destination $stage

# Notices travel with the package; NOTICE.md explains their scope.
$notices = Join-Path $stage 'Licenses'
$onnxNotices = Join-Path $notices 'ONNX-Runtime'
$segFormerNotices = Join-Path $notices 'SegFormer'
New-Item -ItemType Directory -Force $onnxNotices, $segFormerNotices | Out-Null
$onnxNoticeNames = @('LICENSE', 'ThirdPartyNotices.txt')
foreach ($name in $onnxNoticeNames) {
    Copy-Item -LiteralPath (Join-Path $onnxPrefix $name) -Destination $onnxNotices
}
Copy-Item -LiteralPath $onnxPinsPath -Destination $onnxNotices
$segFormerNoticeNames = @('README.md', 'KERAS-MODEL-CARD.md', 'MIT.txt')
foreach ($name in $segFormerNoticeNames) {
    Copy-Item -LiteralPath (Join-Path $root "assets/analysis/$name") -Destination $segFormerNotices
}
foreach ($path in @(
    'LICENSE',
    'NOTICE.md',
    'assets/fonts/OFL.txt',
    'assets/ocio/LICENSE-OpenColorIO.txt',
    'vendor/dng-sdk/LICENSE.txt',
    'vendor/dng-sdk/PROVENANCE.md'
)) {
    $source = Join-Path $root $path
    if (Test-Path $source) {
        Copy-Item $source (Join-Path $notices (Split-Path -Leaf $path))
    }
}
$rawHashes = Get-Content -Raw (Join-Path $root 'native/libraw/dht-hashes.txt') | ConvertFrom-StringData
$rawSource = if ($env:LIBRAW_SOURCE) {
    $env:LIBRAW_SOURCE
} else {
    Join-Path $root ".cache/native-deps/LibRaw-0.22.3-$($rawHashes.patched.Substring(0, 12))-rawspeed-llvm-x64"
}
$rawNotices = Join-Path $notices 'LibRaw'
New-Item -ItemType Directory -Force $rawNotices | Out-Null
foreach ($name in @('COPYRIGHT', 'LICENSE.LGPL', 'LICENSE.CDDL')) {
    Copy-Item (Join-Path $rawSource $name) (Join-Path $rawNotices $name)
}
foreach ($name in @('dht.patch', 'dht-hashes.txt')) {
    Copy-Item (Join-Path $root "native/libraw/$name") (Join-Path $rawNotices $name)
}
$rs = Get-Content -Raw (Join-Path $root 'native/rawspeed/pins.txt') | ConvertFrom-StringData
$rawspeed = Join-Path $root ".cache/native-deps/rawspeed-$($rs.revision.Substring(0, 12))-windows-x64"
$rsNotices = Join-Path $notices 'RawSpeed'
$ompNotices = Join-Path $notices 'OpenMP'
New-Item -ItemType Directory -Force $rsNotices, $ompNotices | Out-Null
foreach ($name in @('RawSpeed-LGPL.txt', 'RawSpeed-credits.txt', 'cameras.xml', 'pugixml.txt')) {
    Copy-Item (Join-Path $rawspeed "share/licenses/$name") (Join-Path $rsNotices $name)
}
$dhtNotices = Join-Path $notices 'DHT'
New-Item -ItemType Directory -Force $dhtNotices | Out-Null
foreach ($path in @('native/dht/README.md', 'native/dht/LICENSE.LGPL-2.1', 'src/raw/demosaic/dht.wgsl')) {
    Copy-Item (Join-Path $root $path) $dhtNotices
}
Copy-Item (Join-Path $root 'native/rawspeed/pins.txt') $rsNotices
Copy-Item (Join-Path $openmp 'share/licenses/LLVM-OpenMP.txt') $ompNotices
$spektrafilm = Join-Path $notices 'spektrafilm'
New-Item -ItemType Directory -Force $spektrafilm | Out-Null
foreach ($name in @('LICENSE-code.txt', 'LICENSE-data.txt', 'NOTICE-Academy-APD.txt', 'CHANGELOG.txt', 'provenance.json')) {
    Copy-Item (Join-Path $root "assets/spektrafilm/$name") (Join-Path $spektrafilm $name)
}
$papers = Join-Path $notices 'papers'
$monochrome = Join-Path $notices 'monochrome'
New-Item -ItemType Directory -Force $papers, $monochrome | Out-Null
Copy-Item (Join-Path $root 'assets/papers/README.md') (Join-Path $papers 'README.md')
$provenancePath = Join-Path $root 'assets/monochrome/provenance.json'
Copy-Item $provenancePath (Join-Path $monochrome 'provenance.json')
$provenance = Get-Content -Raw $provenancePath | ConvertFrom-Json
foreach ($path in $provenance.inputs.PSObject.Properties.Name) {
    if (($path.StartsWith('assets/papers/') -or $path.StartsWith('assets/negatives/')) -and $path.EndsWith('/source.json')) {
        $target = Join-Path $notices $path.Substring('assets/'.Length)
        New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null
        Copy-Item (Join-Path $root $path) $target
    }
}
$licenses = Join-Path $root 'licenses'
if (Test-Path $licenses) {
    Copy-Item $licenses $notices -Recurse
}

@"
$version — portable Windows build

Run antitone.exe. No installation or VC++ redistributable installation is required; the
runtime DLLs beside the executable satisfy its imports and the editor presents through Vulkan.

Requires a Vulkan-capable GPU driver (current NVIDIA, AMD or Intel drivers). Sources are opened
read-only and projects are plain files.

Licensing and component notices: see Licenses/.
"@ | Set-Content (Join-Path $stage 'README.txt') -Encoding UTF8

# Archive and hash the staged files before executing them, so those reads do not
# compete with file handles opened in response to launching the staged executable.
if (Test-Path $archive) { Remove-Item -Force $archive }
Compress-Archive -Path $stage -DestinationPath $archive -CompressionLevel Optimal

# Exercise the archived package under a fresh path, not just the staging directory.
$relocated = Join-Path $dist ('relocated package ' + [guid]::NewGuid().ToString('N'))
try {
    Expand-Archive -LiteralPath $archive -DestinationPath $relocated
    $unpacked = Join-Path $relocated 'Antitone-windows-x64'
    $files = @('antitone.exe', 'libomp.dll', 'onnxruntime.dll', 'Licenses/ONNX-Runtime/pins.txt')
    $files += @($redistributables | ForEach-Object { Split-Path -Leaf $_ })
    $files += @($onnxNoticeNames | ForEach-Object { "Licenses/ONNX-Runtime/$_" })
    $files += @($segFormerNoticeNames | ForEach-Object { "Licenses/SegFormer/$_" })
    foreach ($name in $files) {
        $expected = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $stage $name)).Hash
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $unpacked $name)).Hash
        if ($expected -ne $actual) { throw "Archived file differs: $name" }
    }
    Assert-PortableApplication -Directory $stage -Version $version
    Assert-PortableApplication -Directory $unpacked -Version $version
} finally {
    if (Test-Path $relocated) { Remove-Item -Recurse -Force $relocated }
}

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
"$hash  $(Split-Path -Leaf $archive)" | Set-Content (Join-Path $dist 'SHA256SUMS') -Encoding ASCII
$size = [math]::Round((Get-Item $archive).Length / 1MB, 1)
Write-Host "Packaged $version -> $archive ($size MB)"
