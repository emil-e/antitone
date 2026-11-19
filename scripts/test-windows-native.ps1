#Requires -Version 5.1
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-native.ps1')

function Assert-NativeTest {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}

$root = Split-Path -Parent $PSScriptRoot
$directory = Join-Path $root ('.cache/windows-native-tests/' + [guid]::NewGuid() + ' ' + [char]0x2713 + ' ! path')
$directory = [IO.Path]::GetFullPath($directory)
New-Item -ItemType Directory -Force (Join-Path $directory 'tool-bin') | Out-Null
$before = @{}
foreach ($name in @('PATH', 'INCLUDE', 'LIB', 'ANTITONE_NATIVE_TEST_MARKER')) {
    $before[$name] = [Environment]::GetEnvironmentVariable($name)
}
try {
    $devcmd = Join-Path $directory 'fake devcmd.cmd'
    Set-Content -Encoding Ascii $devcmd @'
@echo off
set "PATH=%~dp0tool-bin;%PATH%"
set "INCLUDE=%~dp0tool-include"
set "LIB=%~dp0tool-lib"
set "ANTITONE_NATIVE_TEST_MARKER=%*"
exit /b 0
'@
    Set-Content -Encoding Ascii (Join-Path $directory 'inspect environment.ps1') @'
$ErrorActionPreference = 'Stop'
@{
    Path = $env:PATH
    Include = $env:INCLUDE
    Lib = $env:LIB
    Marker = $env:ANTITONE_NATIVE_TEST_MARKER
    Directory = [Environment]::CurrentDirectory
} | ConvertTo-Json | Set-Content -Encoding UTF8 'environment.json'
'@
    Set-Content -Encoding Ascii (Join-Path $directory 'tool-bin/antitone-native-path-probe.cmd') @'
@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0..\inspect environment.ps1"
'@
    $tools = [pscustomobject]@{ DevCmd = $devcmd; Arch = 'x64'; HostArch = 'x64' }
    $added = Join-Path $directory 'added'
    Push-Location $directory
    try {
        $command = "set `"PATH=$added;%PATH%`" && set `"INCLUDE=$added;%INCLUDE%`" && set `"LIB=$added;%LIB%`" && antitone-native-path-probe.cmd"
        Invoke-NativeWindowsCommand $tools $command
        $report = Get-Content -Raw -Encoding UTF8 'environment.json' | ConvertFrom-Json
        Assert-NativeTest ($report.Path.StartsWith("$added;$directory\tool-bin;")) 'Initialized PATH was lost'
        Assert-NativeTest ($report.Include -eq "$added;$directory\tool-include") 'Initialized INCLUDE was lost'
        Assert-NativeTest ($report.Lib -eq "$added;$directory\tool-lib") 'Initialized LIB was lost'
        Assert-NativeTest ($report.Marker -eq '-arch=x64 -host_arch=x64') 'Toolchain arguments were lost'
        Assert-NativeTest ($report.Directory -eq $directory) 'PowerShell working directory was lost'
        Remove-Item 'environment.json'

        $powershell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
        Invoke-NativeWindowsCommand $tools "`"$powershell`" -NoProfile -ExecutionPolicy Bypass -File `"$directory/inspect environment.ps1`""
        Assert-NativeTest (Test-Path 'environment.json') 'Quoted executable command did not run'
        Remove-Item 'environment.json'

        $failed = $false
        try { Invoke-NativeWindowsCommand $tools 'exit /b 23' }
        catch { $failed = $_.Exception.Message -eq 'Native build failed with exit code 23' }
        Assert-NativeTest $failed 'Build error was not propagated'

        Set-Content -Encoding Ascii $devcmd '@exit /b 29'
        $failed = $false
        try { Invoke-NativeWindowsCommand $tools 'echo unexpected>environment.json' }
        catch { $failed = $_.Exception.Message -eq 'Visual Studio environment setup failed with exit code 29' }
        Assert-NativeTest $failed 'Toolchain initialization error was not propagated'
        Assert-NativeTest (-not (Test-Path 'environment.json')) 'Build ran after failed initialization'
    } finally { Pop-Location }

    foreach ($name in $before.Keys) {
        Assert-NativeTest ([Environment]::GetEnvironmentVariable($name) -ceq $before[$name]) "Parent $name changed"
    }
    Write-Host 'Windows native runner: initialized environment, Unicode/space/! paths, working directory and failures passed'
} finally { Remove-Item $directory -Recurse -Force }
