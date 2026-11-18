# امضای دیجیتال بیلد ریلیز — SubSync Studio
#
# استفاده (PowerShell):
#   با گواهی نصب‌شده در مخزن ویندوز (thumbprint):
#     $env:SIGN_CERT_THUMBPRINT = "-thumbprint-"; .\scripts\sign-release.ps1
#   با فایل PFX:
#     $env:SIGN_PFX = "C:\certs\my.pfx"; $env:SIGN_PFX_PASSWORD = "..."; .\scripts\sign-release.ps1
#
# بدون هیچ‌کدام اسکریپت خطای واضح می‌دهد و چیزی امضا نمی‌شود.
# timestamp همیشه سرور عمومی است تا امضا پس از انقضای گواهی هم معتبر بماند.

$ErrorActionPreference = "Stop"

$targets = @(
    "src-tauri\target\release\persian-subtitle-studio.exe",
    "src-tauri\target\release\bundle\nsis\SubSync Studio_0.1.0_x64-setup.exe"
)

# ---- یافتن signtool (Windows SDK) ----
$signtool = $null
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) {
    $kits = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -find "ClickOnce\SignTool\signtool.exe" 2>$null
    if ($kits) { $signtool = $kits[0] }
}
if (-not $signtool) {
    $sdkRoots = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Directory -ErrorAction SilentlyContinue |
        Where-Object { Test-Path (Join-Path $_.FullName "x64\signtool.exe") } |
        Sort-Object Name -Descending
    if ($sdkRoots) { $signtool = Join-Path $sdkRoots[0].FullName "x64\signtool.exe" }
}
if (-not $signtool) { throw "signtool یافت نشد — Windows SDK را نصب کنید." }
Write-Host "signtool: $signtool"

# ---- ساخت آرگومان گواهی از env ----
$certArgs = @()
if ($env:SIGN_CERT_THUMBPRINT) {
    $certArgs = @("/sha1", $env:SIGN_CERT_THUMBPRINT, "/n", "SubSync Studio")
} elseif ($env:SIGN_PFX) {
    if (-not $env:SIGN_PFX_PASSWORD) { throw "SIGN_PFX_PASSWORD لازم است." }
    $certArgs = @("/f", $env:SIGN_PFX, "/p", $env:SIGN_PFX_PASSWORD)
} else {
    throw "هیچ گواهی‌ای معرفی نشده — SIGN_CERT_THUMBPRINT یا SIGN_PFX+SIGN_PFX_PASSWORD را ست کنید. (راهنما: docs/RELEASE.md)"
}

$timestamp = "/tr", "http://timestamp.digicert.com", "/td", "sha256"

foreach ($t in $targets) {
    if (-not (Test-Path $t)) { throw "فایل هدف یافت نشد: $t (اول npm run tauri build)" }
    & $signtool sign /fd sha256 @certArgs @timestamp $t
    if ($LASTEXITCODE -ne 0) { throw "امضای $t ناموفق بود." }
    & $signtool verify /pa /all $t
    if ($LASTEXITCODE -ne 0) { throw "تأیید امضای $t ناموفق بود." }
    Write-Host "امضا شد و تأیید شد: $t"
}
Write-Host "امضای ریلیز کامل است."
