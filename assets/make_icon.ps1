# Final multi-resolution Windows icon generator for NameShot.
#
# Source : assets\icon.png (the project artwork - read-only, never modified)
# Output : assets\icon.ico  (frames 16/24/32/48/64/128/256, 32 bpp RGBA,
#          PNG-compressed, square: the source is square, so aspect ratio and
#          transparency are preserved 1:1 with a high-quality downscale)
#
# Run from anywhere:
#   powershell -ExecutionPolicy Bypass -File assets\make_icon.ps1
#
# build.rs embeds assets\icon.ico into nameshot.exe as RT_ICON id 1; the
# runtime loads that resource for the window classes and the tray icon.
param(
    [string]$Source = "$PSScriptRoot\icon.png",
    [string]$OutIco = "$PSScriptRoot\icon.ico"
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$resolved = (Resolve-Path -LiteralPath $Source).Path
$src = [System.Drawing.Image]::FromFile($resolved)
try {
    if ($src.Width -ne $src.Height) {
        throw "source must be square (got $($src.Width)x$($src.Height))"
    }
    $sizes = @(16, 24, 32, 48, 64, 128, 256)
    $frames = @()
    foreach ($size in $sizes) {
        $bmp = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $g.DrawImage($src, 0, 0, $size, $size)
        $g.Dispose()
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        $frames += ,@{ Size = $size; Data = $ms.ToArray() }
        $ms.Dispose()
    }
}
finally {
    $src.Dispose()
}

$fs = [System.IO.File]::Create($OutIco)
try {
    $bw = New-Object System.IO.BinaryWriter($fs)
    $bw.Write([uint16]0)
    $bw.Write([uint16]1)
    $bw.Write([uint16]$frames.Count)
    $offset = 6 + 16 * $frames.Count
    foreach ($f in $frames) {
        $b = if ($f.Size -ge 256) { [byte]0 } else { [byte]$f.Size }
        $bw.Write($b)
        $bw.Write($b)
        $bw.Write([byte]0)
        $bw.Write([byte]0)
        $bw.Write([uint16]1)
        $bw.Write([uint16]32)
        $bw.Write([uint32]$f.Data.Length)
        $bw.Write([uint32]$offset)
        $offset += $f.Data.Length
    }
    foreach ($f in $frames) {
        $bw.Write($f.Data)
    }
    $bw.Flush()
}
finally {
    $fs.Dispose()
}
$total = (Get-Item -LiteralPath $OutIco).Length
Write-Host "wrote $OutIco ($total bytes, frames: $($sizes -join ' '))"
