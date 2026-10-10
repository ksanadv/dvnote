# Makes a release: .\release.ps1 0.4.0
# Sets the version in both Cargo.toml files, builds and tests locally, commits, tags v0.4.0 and pushes. GitHub Actions
# (.github/workflows/release.yml) then builds the release from the tag and publishes it.
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Use a version like 1.2.3, not '$Version'" }
$tag = "v$Version"
if (git status --porcelain) { throw 'Commit or stash your changes first' }
if (git tag --list $tag) { throw "$tag already exists" }
$branch = git branch --show-current
if ($branch -ne 'main') { throw "Releases are made from main, not '$branch'" }

$pattern = [regex]'(?m)^version\s*=\s*"[^"]+"'
foreach ($f in 'Cargo.toml', 'installer/Cargo.toml') {
    $path = Join-Path $PSScriptRoot $f
    $text = [IO.File]::ReadAllText($path)
    [IO.File]::WriteAllText($path, $pattern.Replace($text, "version = `"$Version`"", 1))
}

# also brings both Cargo.lock files up to the new version
.\build-installer.ps1
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cargo test --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }

git add Cargo.toml Cargo.lock installer/Cargo.toml installer/Cargo.lock
git commit -m "Release $tag"
git tag -a $tag -m "dvnote $Version"
git push origin main $tag
if ($LASTEXITCODE) { exit $LASTEXITCODE }
"Pushed $tag. The release appears at https://github.com/ksanadv/dvnote/releases once the Release workflow finishes."
