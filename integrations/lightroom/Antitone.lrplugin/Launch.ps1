param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$Log,
    [string]$Request
)

$ErrorActionPreference = 'Stop'
$arguments = if ($Request) { '--lightroom "' + $Request + '"' } else { '--lightroom-protocol-version' }
$errors = "$Log.stderr"
try {
    # cmd's redirection can leave a Windows-subsystem child without usable standard handles.
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $Log -RedirectStandardError $errors
    $status = $process.ExitCode
} catch {
    [IO.File]::AppendAllText($Log, $_.ToString() + [Environment]::NewLine)
    $status = 1
}
if ([IO.File]::Exists($errors)) {
    $output = [IO.File]::Open($Log, [IO.FileMode]::Append)
    try {
        $inputFile = [IO.File]::OpenRead($errors)
        try { $inputFile.CopyTo($output) } finally { $inputFile.Dispose() }
    } finally { $output.Dispose() }
    [IO.File]::Delete($errors)
}
exit $status
