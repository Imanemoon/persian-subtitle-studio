# ساخت کارت پیش‌نمایش اجتماعی (Social Preview) — 1280×630
# از اسکرین‌شات‌های واقعی برنامه + متن انگلیسی (GDI+ شکل‌دهی فارسی ندارد —
# متن کارت عمداً لاتین است تا رندر تضمین‌شده باشد)
# Usage: powershell -NoProfile -File scripts\make-social-preview.ps1

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$W = 1280; $H = 630
$root = Split-Path -Parent $PSScriptRoot

$card = New-Object System.Drawing.Bitmap($W, $H)
$g = [System.Drawing.Graphics]::FromImage($card)
$g.SmoothingMode = 'AntiAlias'
$g.TextRenderingHint = 'AntiAliasGridFit'
$g.InterpolationMode = 'HighQualityBicubic'

# ---- پس‌زمینه: گرادیان تیره هم‌خانواده با تم برنامه ----
$rect = New-Object System.Drawing.Rectangle(0, 0, $W, $H)
$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    $rect,
    [System.Drawing.Color]::FromArgb(255, 9, 10, 12),
    [System.Drawing.Color]::FromArgb(255, 18, 24, 22),
    35.0)
$g.FillRectangle($brush, $rect)

# ---- خط accent سبز بالا ----
$accent = [System.Drawing.Color]::FromArgb(255, 16, 185, 129)
$accentDim = [System.Drawing.Color]::FromArgb(90, 16, 185, 129)
$g.FillRectangle((New-Object System.Drawing.SolidBrush($accent)), 0, 0, $W, 6)

# ---- اسکرین‌شات‌ها با قاب ----
function Draw-Shot($path, $x, $y, $w, $h) {
    if (-not (Test-Path $path)) { return }
    $img = [System.Drawing.Image]::FromFile($path)
    $frame = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 34, 36, 40))
    $g.FillRectangle($frame, $x - 2, $y - 2, $w + 4, $h + 4)
    $g.DrawImage($img, $x, $y, $w, $h)
    $img.Dispose()
}

Draw-Shot (Join-Path $root "docs\img\pipeline.png") 620 90 620 380
Draw-Shot (Join-Path $root "docs\img\setup.png")   700 170 520 320

# ---- متن ----
$white  = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 240, 242, 245))
$gray   = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 161, 168, 178))
$green  = New-Object System.Drawing.SolidBrush($accentDim)
$greenS = New-Object System.Drawing.SolidBrush($accent)

$fTitle  = New-Object System.Drawing.Font("Segoe UI", 44, [System.Drawing.FontStyle]::Bold)
$fTag    = New-Object System.Drawing.Font("Segoe UI", 20, [System.Drawing.FontStyle]::Regular)
$fSub    = New-Object System.Drawing.Font("Segoe UI", 14, [System.Drawing.FontStyle]::Regular)
$fBadge  = New-Object System.Drawing.Font("Segoe UI", 13, [System.Drawing.FontStyle]::Bold)

$g.DrawString("SubSync Studio", $fTitle, $white, 56, 96)
$g.DrawString("Persian subtitles, fully automated", $fTag, $gray, 60, 186)

$bullets = @(
    "Extract & download Persian subtitles",
    "Sync timing with FFsubsync",
    "Auto-installs all prerequisites on first run",
    "Tauri 2 + Rust + React  |  MIT"
)
$y = 250
foreach ($b in $bullets) {
    $g.FillEllipse($greenS, 64, $y + 9, 10, 10)
    $g.DrawString($b, $fSub, $white, 86, $y)
    $y += 34
}

# ---- بج شکل‌دار (MIT + Windows) ----
$badge = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 22, 26, 28))
$g.FillRectangle($badge, 56, 420, 180, 40)
$g.FillRectangle($green, 56, 420, 6, 40)
$g.DrawString("MIT License", $fBadge, $white, 74, 430)
$g.FillRectangle($badge, 250, 420, 220, 40)
$g.FillRectangle($green, 250, 420, 6, 40)
$g.DrawString("Windows 10 / 11 x64", $fBadge, $white, 268, 430)

# ---- نوار پایین ----
$g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 13, 15, 17))), 0, $H - 46, $W, 46)
$g.DrawString("github.com/Imanemoon/persian-subtitle-studio", $fBadge, $greenS, 56, $H - 38)
$g.DrawString("Free & open source", $fSub, $gray, 620, $H - 36)

$g.Dispose()
$out = Join-Path $root "docs\img\social-preview.png"
$card.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$card.Dispose()
Write-Host "saved: $out"
