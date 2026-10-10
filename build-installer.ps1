$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$pattern = '^version\s*=\s*"([^"]+)"'
$appVersion = (Select-String -Path Cargo.toml -Pattern $pattern | Select-Object -First 1).Matches.Groups[1].Value
$setupVersion = (Select-String -Path installer/Cargo.toml -Pattern $pattern | Select-Object -First 1).Matches.Groups[1].Value
if ($appVersion -ne $setupVersion) { throw "Cargo.toml is $appVersion but installer/Cargo.toml is $setupVersion; keep them equal" }

cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$target = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$env:DVNOTE_EXE = Join-Path $target 'release\dvnote.exe'

# the installer is its own workspace, so point it at the same target directory
cargo build --release --manifest-path installer/Cargo.toml --target-dir $target
if ($LASTEXITCODE) { exit $LASTEXITCODE }

# release asset names; src/update.rs downloads the setup by the same pattern
$name = "dvnote-$appVersion-windows-x64"
New-Item -ItemType Directory -Force dist | Out-Null
Remove-Item dist\dvnote-* -ErrorAction SilentlyContinue
Copy-Item (Join-Path $target 'release\dvnote-setup.exe') "dist\$name-setup.exe"
Copy-Item (Join-Path $target 'release\dvnote.exe') "dist\$name-portable.exe"
Get-Item dist\$name-* | ForEach-Object { '{0}  {1:N1} MB' -f $_.FullName, ($_.Length / 1MB) }
