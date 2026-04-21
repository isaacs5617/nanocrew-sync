# Downloads the WinFsp MSI for bundling with the NanoCrew Sync installer.
# Run once before building a release: pwsh -File download-winfsp.ps1
$version = "2.1.25156"
$url = "https://github.com/winfsp/winfsp/releases/download/v2.1/winfsp-$version.msi"
$dest = "$PSScriptRoot\winfsp.msi"
if (Test-Path $dest) { Write-Host "Already downloaded: $dest"; exit 0 }
Write-Host "Downloading WinFsp $version..."
Invoke-WebRequest -Uri $url -OutFile $dest
Write-Host "Saved to $dest"
