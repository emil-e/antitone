# Select the CRT independently of Visual Studio's runtime-set branding. OpenMP
# comes from our pinned LLVM build, never from a Visual Studio redistributable.
function Get-LlvmOpenMPRuntime {
    param([Parameter(Mandatory = $true)][string]$Prefix)
    $file = Join-Path $Prefix 'bin/libomp.dll'
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Missing pinned OpenMP runtime $file" }
    (Get-Item -LiteralPath $file).FullName
}

function Assert-LlvmOpenMPImports {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Lines)
    $imports = @($Lines | ForEach-Object { $_.Trim() } | Where-Object { $_ -match '\.dll$' })
    $other = @($imports | Where-Object { $_ -match '^(vcomp|libiomp|libgomp|libomp\d)' })
    if ($imports -notcontains 'libomp.dll' -or $other.Count -ne 0) {
        throw "Expected only pinned libomp.dll for OpenMP; imports: $imports"
    }
}

function Get-VcRuntimeFiles {
    param([Parameter(Mandatory = $true)][string]$RedistRoot)

    # Ignore aliases such as v143 and compare actual versions numerically.
    $redist = Get-ChildItem -LiteralPath $RedistRoot -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending |
        Select-Object -First 1
    if (-not $redist) { throw 'No Visual C++ redistributable directory found.' }
    $runtimeRoot = Join-Path $redist.FullName 'x64'
    $sets = [ordered]@{
        CRT = @('msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
    }
    foreach ($kind in $sets.Keys) {
        $directory = Get-ChildItem -LiteralPath $runtimeRoot -Directory |
            Where-Object { $_.Name -match ('^Microsoft\.VC\d+\.{0}$' -f $kind) } |
            Sort-Object { [int]([regex]::Match($_.Name, 'VC(\d+)').Groups[1].Value) } -Descending |
            Select-Object -First 1
        if (-not $directory) { throw "No Visual C++ $kind runtime set under $runtimeRoot" }
        foreach ($name in $sets[$kind]) {
            $file = Join-Path $directory.FullName $name
            if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
                throw "Missing runtime file $file"
            }
            (Get-Item -LiteralPath $file).FullName
        }
    }
}
