param(
  [string]$Tag
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if (-not $PSBoundParameters.ContainsKey('Tag') -and $env:GITHUB_REF_TYPE -eq 'tag') {
  $Tag = $env:GITHUB_REF_NAME
}

$packageJson = Get-Content -LiteralPath (Join-Path $root 'package.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$tauriConfig = Get-Content -LiteralPath (Join-Path $root 'src-tauri\tauri.conf.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$metadata = cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $root 'Cargo.toml') | ConvertFrom-Json

$expected = [string]$packageJson.version
$versions = [ordered]@{
  'package.json' = $expected
  'tauri.conf.json' = [string]$tauriConfig.version
}

foreach ($package in $metadata.packages) {
  $versions["Cargo package $($package.name)"] = [string]$package.version
}

$mismatches = @($versions.GetEnumerator() | Where-Object { $_.Value -ne $expected })
if ($mismatches.Count -gt 0) {
  $details = ($mismatches | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '
  throw "PowerShift version mismatch. Expected $expected; found $details"
}

if ($Tag) {
  if ($Tag -notmatch '^v(?<version>\d+\.\d+\.\d+)(?<suffix>-[0-9A-Za-z.-]+)?$') {
    throw "Release tag '$Tag' must use vMAJOR.MINOR.PATCH or vMAJOR.MINOR.PATCH-suffix."
  }
  if ($Matches.version -ne $expected) {
    throw "Release tag '$Tag' does not match application version $expected."
  }
}

Write-Output "PowerShift version verified: $expected"
