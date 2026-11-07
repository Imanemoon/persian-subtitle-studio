# ساخت آیکون پایه SubSync Studio (1024x1024 PNG)
# خروجی سپس با `npx tauri icon` به همه سایزها تبدیل می‌شود.
param(
    [string]$OutPath = "$PSScriptRoot\app-icon.png"
)

Add-Type -AssemblyName System.Drawing

$size = 1024
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit

function New-RoundRectPath {
    param([float]$x, [float]$y, [float]$w, [float]$h, [float]$r)
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $p.AddArc($x, $y, 2 * $r, 2 * $r, 180, 90)
    $p.AddArc($x + $w - 2 * $r, $y, 2 * $r, 2 * $r, 270, 90)
    $p.AddArc($x + $w - 2 * $r, $y + $h - 2 * $r, 2 * $r, 2 * $r, 0, 90)
    $p.AddArc($x, $y + $h - 2 * $r, 2 * $r, 2 * $r, 90, 90)
    $p.CloseFigure()
    return $p
}

# پس‌زمینه شفاف + مربع گرد تیره
$bgPath = New-RoundRectPath 16 16 ($size - 32) ($size - 32) 220
$bgBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 15, 15, 18))
$g.FillPath($bgBrush, $bgPath)

# حرف S (سرمایه‌ی نام برنامه)
$font = New-Object System.Drawing.Font("Segoe UI", 460, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
$white = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 244, 244, 245))
$sf = New-Object System.Drawing.StringFormat
$sf.Alignment = [System.Drawing.StringAlignment]::Center
$sf.LineAlignment = [System.Drawing.StringAlignment]::Center
$g.DrawString("S", $font, $white, (New-Object System.Drawing.RectangleF(0, -60, $size, $size)), $sf)

# دو خط زیرنویس (هویت اصلی برنامه: Caption Bars)
$accent = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 16, 185, 129))
$softWhite = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 190, 190, 195))

$bar1 = New-RoundRectPath 200 660 624 88 44
$g.FillPath($accent, $bar1)
$bar2 = New-RoundRectPath 200 788 424 72 36
$g.FillPath($softWhite, $bar2)

$bmp.Save($OutPath, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()
Write-Host "Icon written to $OutPath"
