param(
    [switch]$NoDesktop
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$InstallDir = Join-Path $env:LOCALAPPDATA "SoflePlus2Viewer"
$ExeSource = Join-Path $RepoRoot "target\release\sofle-plus2-viewer.exe"
$ExeTarget = Join-Path $InstallDir "sofle-plus2-viewer.exe"
$StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
$StartMenuShortcut = Join-Path $StartMenuDir "Sofle Plus 2 Viewer.lnk"
$DesktopShortcut = Join-Path ([Environment]::GetFolderPath("Desktop")) "Sofle Plus 2 Viewer.lnk"

Push-Location $RepoRoot
try {
    cargo build --release
}
finally {
    Pop-Location
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
New-Item -ItemType Directory -Force -Path $StartMenuDir | Out-Null
Copy-Item -Force $ExeSource $ExeTarget

function New-SofleShortcut {
    param(
        [Parameter(Mandatory = $true)][string]$Path
    )

    $Shell = New-Object -ComObject WScript.Shell
    $Shortcut = $Shell.CreateShortcut($Path)
    $Shortcut.TargetPath = $ExeTarget
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.Description = "Live XCMKB Sofle Plus 2 keyboard viewer"
    $Shortcut.Save()
}

New-SofleShortcut -Path $StartMenuShortcut
if (-not $NoDesktop) {
    New-SofleShortcut -Path $DesktopShortcut
}

Write-Host "Installed $ExeTarget"
Write-Host "Installed Start Menu shortcut: $StartMenuShortcut"
if (-not $NoDesktop) {
    Write-Host "Installed Desktop shortcut: $DesktopShortcut"
}
Write-Host "Close Vial before running the viewer, because both use the raw HID interface."
