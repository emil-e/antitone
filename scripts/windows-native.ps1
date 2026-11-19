# Shared discovery and checked downloads for the Windows RAW dependency builds.
function Get-NativeWindowsTools {
    param([string]$Root, [string]$Arch = 'x64', [string]$HostArch = 'x64')
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw 'Visual Studio C++ Build Tools with vswhere.exe is required.' }
    $component = if ($Arch -eq 'arm64') {
        'Microsoft.VisualStudio.Component.VC.Tools.ARM64'
    } else { 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' }
    $installation = & $vswhere -latest -products * -requires $component -property installationPath |
        Select-Object -First 1
    if (-not $installation) { throw 'No Visual Studio installation with the requested C++ tools was found.' }
    $devcmd = Join-Path $installation 'Common7/Tools/VsDevCmd.bat'
    $clang = @('VC/Tools/Llvm/x64/bin/clang++.exe', 'VC/Tools/Llvm/bin/clang++.exe') |
        ForEach-Object { Join-Path $installation $_ } | Where-Object { Test-Path $_ } |
        Select-Object -First 1
    if (-not $clang) { throw 'Install the Visual Studio C++ Clang tools for Windows component.' }
    $cmake = if ($env:CMAKE) { $env:CMAKE } else {
        $local = Get-ChildItem (Join-Path $Root '.cache/tools/cmake') -Filter cmake.exe -Recurse -ErrorAction SilentlyContinue |
            Select-Object -First 1
        if ($local) { $local.FullName } else { (Get-Command cmake.exe -ErrorAction Stop).Source }
    }
    $ninja = Join-Path $Root '.cache/tools/ninja/ninja.exe'
    if (-not (Test-Path $ninja)) { $ninja = (Get-Command ninja.exe -ErrorAction Stop).Source }
    $target = if ($Arch -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
    [pscustomobject]@{ DevCmd = $devcmd; Clang = $clang; CMake = $cmake; Ninja = $ninja
        Target = $target; Arch = $Arch; HostArch = $HostArch }
}

function Invoke-NativeWindowsCommand {
    param($Tools, [string]$Command)
    # Initialize first: cmd expands %PATH% across an entire && chain before executing it.
    $setup = New-Object System.Diagnostics.ProcessStartInfo
    $setup.FileName = $env:ComSpec
    $setup.Arguments = "/d /v:off /u /s /c `"call `"$($Tools.DevCmd)`" -arch=$($Tools.Arch) -host_arch=$($Tools.HostArch) >nul && set`""
    $setup.WorkingDirectory = (Get-Location).ProviderPath
    $setup.UseShellExecute = $false
    $setup.RedirectStandardOutput = $true
    $setup.StandardOutputEncoding = [System.Text.Encoding]::Unicode
    $process = [System.Diagnostics.Process]::Start($setup)
    try {
        $environment = $process.StandardOutput.ReadToEnd()
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "Visual Studio environment setup failed with exit code $($process.ExitCode)" }
    } finally { $process.Dispose() }

    $build = New-Object System.Diagnostics.ProcessStartInfo
    $build.FileName = $env:ComSpec
    $build.Arguments = "/d /v:off /s /c `"$Command`""
    $build.WorkingDirectory = (Get-Location).ProviderPath
    $build.UseShellExecute = $false
    $build.EnvironmentVariables.Clear()
    foreach ($line in ($environment -split "`r?`n")) {
        $separator = $line.IndexOf('=')
        if ($separator -gt 0) {
            $build.EnvironmentVariables[$line.Substring(0, $separator)] = $line.Substring($separator + 1)
        }
    }
    $process = [System.Diagnostics.Process]::Start($build)
    try {
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) { throw "Native build failed with exit code $($process.ExitCode)" }
    } finally { $process.Dispose() }
}

function Get-PinnedNativeSource {
    param([string]$Archive, [string]$Url, [string]$Sha256, [string]$Destination,
          [string]$Marker, [string[]]$Members = @())
    New-Item -ItemType Directory -Force (Split-Path -Parent $Archive) | Out-Null
    if (-not (Test-Path $Archive)) {
        Invoke-WebRequest -Uri $Url -OutFile "$Archive.tmp" -UseBasicParsing
        Move-Item "$Archive.tmp" $Archive
    }
    $actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    if ($actual -ne $Sha256) { throw "$Archive SHA-256 mismatch: $actual, expected $Sha256" }
    if (-not (Test-Path (Join-Path $Destination $Marker))) {
        $temporary = "$Destination.tmp"
        if (Test-Path $temporary) { Remove-Item $temporary -Recurse -Force }
        New-Item -ItemType Directory -Force $temporary | Out-Null
        & tar.exe -xf $Archive -C $temporary --strip-components=1 @Members
        if ($LASTEXITCODE -ne 0) { throw "Could not extract $Archive" }
        if (-not (Test-Path (Join-Path $temporary $Marker))) { throw "Missing $Marker in $Archive" }
        if (Test-Path $Destination) { Remove-Item $Destination -Recurse -Force }
        Move-Item $temporary $Destination
    }
}
