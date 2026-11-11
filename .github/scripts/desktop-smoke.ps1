# Runs the offline native smoke test of a Windows desktop build and fails the
# step when it times out or exits with an error. The data directory must be new.
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$DataDir
)
$start = [System.Diagnostics.ProcessStartInfo]::new()
$start.FileName = (Resolve-Path $Exe).Path
$start.Arguments = "--data-dir `"$DataDir`" --smoke-test"
$start.UseShellExecute = $false
$gui = [System.Diagnostics.Process]::Start($start)
if (-not $gui.WaitForExit(120000)) {
    $gui.Kill($true)
    throw "Desktop smoke test timed out: $Exe"
}
$exitCode = $gui.ExitCode
$gui.Dispose()
if ($exitCode -ne 0) { throw "Desktop smoke test failed with $exitCode`: $Exe" }
Write-Host "Desktop smoke test passed: $Exe"
