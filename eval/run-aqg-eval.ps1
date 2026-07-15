Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$srcTauriDir = Join-Path $repoRoot 'src-tauri'
$defaultNote = Join-Path $repoRoot 'docs\evaluation_notes\Photosynthesis.md'
$outputDir = Join-Path $repoRoot 'eval\output'

$notePath = $defaultNote
if ($args.Count -ge 1 -and -not [string]::IsNullOrWhiteSpace($args[0])) {
    $notePath = $args[0]
}

if (-not (Test-Path $notePath)) {
    throw "Evaluation note not found: $notePath"
}

Push-Location $srcTauriDir
try {
    cargo run --bin aqg_eval -- --note "$notePath" --output-dir "$outputDir"
}
finally {
    Pop-Location
}