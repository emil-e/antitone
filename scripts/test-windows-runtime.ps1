#Requires -Version 5.1
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'windows-runtime.ps1')

$root = Join-Path (Split-Path -Parent $PSScriptRoot) ('.cache/runtime tests ' + [guid]::NewGuid().ToString('N'))
$names = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll', 'vcomp140.dll')

function Add-Layout([string]$Directory, [string]$Version, [int]$Brand) {
    $paths = foreach ($name in $names) {
        $kind = if ($name -eq 'vcomp140.dll') { 'OpenMP' } else { 'CRT' }
        $set = Join-Path $Directory "$Version/x64/Microsoft.VC$Brand.$kind"
        New-Item -ItemType Directory -Force -Path $set | Out-Null
        $file = Join-Path $set $name
        Set-Content -LiteralPath $file -Value "$Version $Brand $name"
        $file
    }
    return $paths
}

function Assert-Selection([string]$Directory, [string[]]$Expected) {
    $actual = @(Get-VcRuntimeFiles -RedistRoot $Directory)
    if ($actual.Count -ne 4 -or (Compare-Object $Expected $actual)) {
        throw "Wrong runtime selection: $actual; expected $Expected"
    }
}

function Assert-Failure([string]$Directory) {
    $failed = $false
    try { Get-VcRuntimeFiles -RedistRoot $Directory | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw "Incomplete runtime layout was accepted: $Directory" }
}

try {
    $old = Join-Path $root 'VS 2022'
    $expected = @(Add-Layout $old '14.36.32532' 143)
    Assert-Selection $old $expected

    $current = Join-Path $root 'VS 2026'
    Add-Layout $current '14.9.10000' 143 | Out-Null
    Add-Layout $current 'v143' 143 | Out-Null
    $expected = @(Add-Layout $current '14.51.36231' 145)
    Assert-Selection $current $expected

    # Extra runtime-set brands cannot make selection depend on enumeration order.
    Add-Layout $current '14.51.36231' 143 | Out-Null
    Assert-Selection $current $expected

    # Fail rather than silently ship an older or incomplete runtime.
    Remove-Item -LiteralPath $expected[0]
    Assert-Failure $current
    Remove-Item -Recurse -Force -LiteralPath (Split-Path -Parent $expected[0])
    Remove-Item -Recurse -Force -LiteralPath (Join-Path $current '14.51.36231/x64/Microsoft.VC143.CRT')
    Assert-Failure $current
    Remove-Item -Recurse -Force -LiteralPath (Join-Path $old '14.36.32532/x64/Microsoft.VC143.OpenMP')
    Assert-Failure $old
    Write-Host 'Windows runtime selection: all tests passed'
} finally {
    if (Test-Path -LiteralPath $root) { Remove-Item -Recurse -Force -LiteralPath $root }
}
