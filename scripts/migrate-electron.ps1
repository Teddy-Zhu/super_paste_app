$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
cargo run --manifest-path (Join-Path $projectRoot 'tools/migrate-electron/Cargo.toml') -- @args
