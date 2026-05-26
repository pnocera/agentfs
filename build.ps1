[CmdletBinding()]
param(
    [ValidateSet("debug", "release", "dist")]
    [string]$Profile = "release",

    [string]$Target,

    [switch]$NoDefaultFeatures,

    [switch]$Clean
)

$ErrorActionPreference = "Stop"

$RepoRoot = $PSScriptRoot
$CliManifest = Join-Path $RepoRoot "cli\Cargo.toml"
$DistDir = Join-Path $RepoRoot "dist"

if (-not (Test-Path -LiteralPath $CliManifest)) {
    throw "Could not find CLI Cargo manifest at $CliManifest"
}

if ($Clean -and (Test-Path -LiteralPath $DistDir)) {
    Remove-Item -LiteralPath $DistDir -Recurse -Force
}

New-Item -ItemType Directory -Path $DistDir -Force | Out-Null

$CargoArgs = @("build", "--manifest-path", $CliManifest)
if ($Profile -eq "release") {
    $CargoArgs += "--release"
} elseif ($Profile -eq "dist") {
    $CargoArgs += @("--profile", "dist")
}

if ($Target) {
    $CargoArgs += @("--target", $Target)
}

if ($NoDefaultFeatures) {
    $CargoArgs += "--no-default-features"
}

Write-Host "Building agentfs..."
Write-Host "cargo $($CargoArgs -join ' ')"
& cargo @CargoArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$TargetRoot = Join-Path $RepoRoot "cli\target"
if ($Target) {
    $TargetRoot = Join-Path $TargetRoot $Target
}

$ProfileDir = if ($Profile -eq "debug") { "debug" } else { $Profile }
$ExeName = if ($IsWindows -or $env:OS -eq "Windows_NT") { "agentfs.exe" } else { "agentfs" }
$BuiltExe = Join-Path (Join-Path $TargetRoot $ProfileDir) $ExeName

if (-not (Test-Path -LiteralPath $BuiltExe)) {
    throw "Build completed, but expected binary was not found at $BuiltExe"
}

$OutExe = Join-Path $DistDir $ExeName
Copy-Item -LiteralPath $BuiltExe -Destination $OutExe -Force

Write-Host "Built $OutExe"
