# Select redistributable files independently of Visual Studio's runtime-set branding.
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
        CRT = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
        OpenMP = @('vcomp140.dll')
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
