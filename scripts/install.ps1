Param(
  [string]$Repo = "GM-Zhou/codex-manager",

  [string]$Version = "latest",
  [string]$InstallDir = "$HOME\AppData\Local\bin"
)

$ErrorActionPreference = "Stop"

$BinName = "codexm.exe"
$AssetName = "codexm-windows-amd64.exe"

function Write-Info($msg) {
  Write-Host "[codexm] $msg"
}

if ($Repo -notmatch "^[^/]+/[^/]+$") {
  throw "Repo must be in owner/repo format, e.g. user/codex-manager"
}

if ($Version -eq "latest") {
  $releasePath = "releases/latest/download"
}
else {
  $releasePath = "releases/download/$Version"
}

$baseUrl = "https://github.com/$Repo/$releasePath"
$downloadUrl = "$baseUrl/$AssetName"

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

$tmpDir = Join-Path $env:TEMP ("codexm-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

try {
  $tmpBinary = Join-Path $tmpDir $AssetName
  Write-Info "Downloading $downloadUrl"
  Invoke-WebRequest -Uri $downloadUrl -OutFile $tmpBinary

  $targetPath = Join-Path $InstallDir $BinName
  Copy-Item -Path $tmpBinary -Destination $targetPath -Force

  Write-Info "Installed to: $targetPath"
  if (($env:Path -split ';') -contains $InstallDir) {
    Write-Info "Run: codexm --help"
  }
  else {
    Write-Warning "$InstallDir is not in PATH."
    Write-Host "Add it for current session:"
    Write-Host "  `$env:Path = `"$InstallDir;`$env:Path`""
  }
}
finally {
  Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}
