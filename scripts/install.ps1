<#
.SYNOPSIS
  Installs or updates Speechify from GitHub releases.

.DESCRIPTION
  irm https://raw.githubusercontent.com/Sumedhbhat/speechify/main/scripts/install.ps1 | iex

  Channels (see docs/releases.md):
    stable   the latest stable release (default)
    nightly  the newest prerelease, built from main
    v0.1.2   a specific release

  When piping into iex, pick the channel with $env:SPEECHIFY_CHANNEL = "nightly".
  Your recordings, corrections and settings live in %APPDATA%\speechify and
  are never touched by installs or updates.
#>
param(
    [string]$Channel = $(if ($env:SPEECHIFY_CHANNEL) { $env:SPEECHIFY_CHANNEL } else { "stable" }),
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\Speechify",
    [switch]$Autostart
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"  # Invoke-WebRequest is 10x slower with the progress bar
$repo = "Sumedhbhat/speechify"
$api = "https://api.github.com/repos/$repo/releases"
$headers = @{ "User-Agent" = "speechify-installer"; "Accept" = "application/vnd.github+json" }
$download = @{ "User-Agent" = "speechify-installer" }

switch -Regex ($Channel) {
    "^stable$"  { $release = Invoke-RestMethod "$api/latest" -Headers $headers }
    "^nightly$" {
        $release = Invoke-RestMethod "$api`?per_page=30" -Headers $headers |
            Where-Object { $_.prerelease } | Select-Object -First 1
        if (-not $release) { throw "No nightly release found." }
    }
    "^v\d"      { $release = Invoke-RestMethod "$api/tags/$Channel" -Headers $headers }
    default     { throw "Unknown channel '$Channel'. Use stable, nightly, or a tag like v0.1.2." }
}
$version = $release.tag_name.TrimStart("v")
Write-Host "Installing Speechify $version ($Channel)"

$zipAsset = $release.assets | Where-Object { $_.name -eq "speechify-$version.zip" }
$sumsAsset = $release.assets | Where-Object { $_.name -eq "SHA256SUMS" }
if (-not $zipAsset -or -not $sumsAsset) { throw "Release $($release.tag_name) is missing its assets." }

$tmp = Join-Path ([IO.Path]::GetTempPath()) "speechify-$([Guid]::NewGuid())"
New-Item -ItemType Directory $tmp | Out-Null
try {
    $zip = Join-Path $tmp $zipAsset.name
    Invoke-WebRequest $zipAsset.browser_download_url -OutFile $zip -Headers $download
    $sums = (Invoke-WebRequest $sumsAsset.browser_download_url -Headers $download -UseBasicParsing).Content
    if ($sums -is [byte[]]) { $sums = [Text.Encoding]::UTF8.GetString($sums) }
    $expected = ($sums -split "`n" | Where-Object { $_ -match [regex]::Escape($zipAsset.name) }) -replace "\s.*$", ""
    $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $actual -ne $expected.Trim().ToLower()) { throw "Checksum mismatch for $($zipAsset.name)." }

    Expand-Archive $zip -DestinationPath $tmp
    if (Test-Path $InstallDir) { Remove-Item $InstallDir -Recurse -Force }
    New-Item -ItemType Directory (Split-Path $InstallDir) -Force | Out-Null
    Move-Item (Join-Path $tmp "speechify-$version") $InstallDir
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    Write-Host "Installing uv (the Python package manager Speechify runs on)"
    if (Get-Command winget -ErrorAction SilentlyContinue) {
        winget install --id astral-sh.uv -e --silent --accept-source-agreements --accept-package-agreements
    } else {
        Invoke-RestMethod https://astral.sh/uv/install.ps1 | Invoke-Expression
    }
    $env:Path = [Environment]::GetEnvironmentVariable("Path", "User") + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")
}

Push-Location $InstallDir
try {
    uv sync --frozen --no-dev
    if ($LASTEXITCODE -ne 0) { throw "uv sync failed." }
    $installed = uv run --frozen --no-dev speechify --version
} finally {
    Pop-Location
}

$shell = New-Object -ComObject WScript.Shell
$targets = @([Environment]::GetFolderPath("Programs"))
if ($Autostart) { $targets += [Environment]::GetFolderPath("Startup") }
foreach ($dir in $targets) {
    $link = $shell.CreateShortcut((Join-Path $dir "Speechify.lnk"))
    $link.TargetPath = Join-Path $InstallDir "start.bat"
    $link.WorkingDirectory = $InstallDir
    $link.WindowStyle = 7  # minimized
    $link.Save()
}

Write-Host "$installed installed to $InstallDir"
Write-Host "Start it from the Start menu (Speechify), then hold Right Ctrl to talk."
Write-Host "The first start downloads the speech model (about 1.6 GB)."
