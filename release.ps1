# Publish a new version:  .\release.ps1 0.2.0
# Bumps the version everywhere, commits, tags and pushes. GitHub then builds the
# installer and publishes it; installed copies pick the update up on next launch.
param([Parameter(Mandatory = $true)][string]$Version)
$ErrorActionPreference = "Stop"
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version must look like 1.2.3" }
Set-Location $PSScriptRoot

function Set-Version($path, $pattern, $replacement) {
    $text = [IO.File]::ReadAllText((Resolve-Path $path))
    $new = [regex]::new($pattern, 'Multiline').Replace($text, $replacement, 1)
    if ($new -eq $text) { Write-Host "  (no change in $path)" }
    [IO.File]::WriteAllText((Resolve-Path $path), $new)
}
Set-Version "package.json" '"version": "[^"]+"' "`"version`": `"$Version`""
Set-Version "src-tauri/tauri.conf.json" '"version": "[^"]+"' "`"version`": `"$Version`""
Set-Version "src-tauri/Cargo.toml" '^version = "[^"]+"' "version = `"$Version`""

npm install --package-lock-only --no-audit --no-fund | Out-Null
Push-Location src-tauri; cargo update -p lingoreel --offline 2>$null | Out-Null; Pop-Location

git add -A
git commit -m "Release v$Version"
git tag "v$Version"
git push origin HEAD
git push origin "v$Version"
Write-Host "Pushed v$Version. Watch the build at https://github.com/JustSad42/LingoReel/actions"
