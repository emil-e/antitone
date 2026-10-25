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
Get-VcRuntimeFiles -RedistRoot (Join-Path $installation 'VC/Redist/MSVC') | ForEach-Object {
    Copy-Item -LiteralPath $_ -Destination $stage
}

foreach ($name in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'vcomp140.dll')) {
    if (-not (Test-Path (Join-Path $stage $name))) { throw "Runtime file $name was not staged" }
}

# Notices travel with the package; NOTICE.md explains their scope.
$notices = Join-Path $stage 'Licenses'
New-Item -ItemType Directory -Force $notices | Out-Null
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

# A package that cannot start is not a package: run the staged binary, not the build output.
$smoke = & (Join-Path $stage 'antitone.exe') --version
if ($LASTEXITCODE -ne 0 -or $smoke.Trim() -ne $version) {
    throw "Staged binary failed its smoke test: '$smoke' (exit $LASTEXITCODE)"
}

if (Test-Path $archive) { Remove-Item -Force $archive }
Compress-Archive -Path $stage -DestinationPath $archive -CompressionLevel Optimal

# Exercise the archived package under a fresh path, not just the staging directory.
$relocated = Join-Path $dist ('relocated package ' + [guid]::NewGuid().ToString('N'))
try {
    Expand-Archive -LiteralPath $archive -DestinationPath $relocated
    $unpacked = Join-Path $relocated 'Antitone-windows-x64'
    foreach ($name in @('antitone.exe', 'msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'vcomp140.dll')) {
        $expected = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $stage $name)).Hash
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $unpacked $name)).Hash
        if ($expected -ne $actual) { throw "Archived file differs: $name" }
    }
    Push-Location $unpacked
    try {
        $smoke = & '.\antitone.exe' --version
        if ($LASTEXITCODE -ne 0 -or $smoke.Trim() -ne $version) {
            throw "Relocated package failed its smoke test: '$smoke' (exit $LASTEXITCODE)"
        }
    } finally {
        Pop-Location
    }
} finally {
    if (Test-Path $relocated) { Remove-Item -Recurse -Force $relocated }
}

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
"$hash  $(Split-Path -Leaf $archive)" | Set-Content (Join-Path $dist 'SHA256SUMS') -Encoding ASCII
$size = [math]::Round((Get-Item $archive).Length / 1MB, 1)
Write-Host "Packaged $version -> $archive ($size MB)"
