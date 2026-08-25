param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string]$ReleaseDir
)

$ErrorActionPreference = "Stop"

function Fail([string]$Message) {
  [Console]::Error.WriteLine($Message)
  exit 1
}

if (-not (Test-Path -LiteralPath $ReleaseDir -PathType Container)) {
  Fail "release directory is missing: $ReleaseDir"
}

$nsisDir = Join-Path $ReleaseDir "bundle\nsis"
$setup = $null
if (Test-Path -LiteralPath $nsisDir -PathType Container) {
  $setup = Get-ChildItem -LiteralPath $nsisDir -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "*_x64-setup.exe" } |
    Select-Object -First 1
}
if (-not $setup) {
  Fail "NSIS x64 installer is missing (expected *_x64-setup.exe under bundle/nsis)"
}

$exe = Join-Path $ReleaseDir "desktop-translator.exe"
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
  Fail "native executable is missing: desktop-translator.exe"
}

$bytes = (Get-Item -LiteralPath $exe).Length
$limit = 24 * 1024 * 1024
if ($bytes -le 0) {
  Fail "release executable is empty"
}
if ($bytes -gt $limit) {
  Fail "release executable exceeds compactness budget: $bytes > $limit"
}

$developer = @(
  Get-ChildItem -LiteralPath $ReleaseDir -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match "^(node|python|pythonw|rustc|cargo|pnpm)\.exe$" }
)
if ($developer.Count -ne 0) {
  Fail "release contains a developer-machine runtime: $($developer.Name -join ', ')"
}

$textbooks = @(
  Get-ChildItem -LiteralPath $ReleaseDir -Recurse -File -Filter "starter-en-zh.sqlite3" -ErrorAction SilentlyContinue
)
if ($textbooks.Count -ne 0) {
  Fail "bundled textbook must be embedded once in the executable"
}

Write-Output "bundle audit passed: executable=$bytes bytes installer=$($setup.Name)"
