param(
    [string]$Source = (Join-Path $PSScriptRoot 'icon.png'),
    [string]$Out = (Join-Path $PSScriptRoot 'icon.ico')
)
$ErrorActionPreference = 'Stop'
Write-Host "TEMPORARY placeholder generator: $Source -> $Out (replace icon.ico with the final multi-size .ico when ready)"
Add-Type -AssemblyName System.Drawing

$sizes = @(16, 32, 48, 256)
$src = New-Object System.Drawing.Bitmap($Source)
try {
    $entries = @()
    foreach ($s in $sizes) {
        $bmp = New-Object System.Drawing.Bitmap($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $g.Clear([System.Drawing.Color]::Transparent)
            $dest = New-Object System.Drawing.Rectangle(0, 0, $s, $s)
            $g.DrawImage($src, $dest)
        }
        finally { $g.Dispose() }
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        $entries += @{ Size = $s; Data = $ms.ToArray() }
        $ms.Dispose()
    }
}
finally { $src.Dispose() }

$stream = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($stream)
$bw.Write([uint16]0)
$bw.Write([uint16]1)
$bw.Write([uint16]$entries.Count)
$offset = 6 + 16 * $entries.Count
foreach ($e in $entries) {
    $dim = if ($e.Size -ge 256) { [byte]0 } else { [byte]$e.Size }
    $bw.Write($dim)
    $bw.Write($dim)
    $bw.Write([byte]0)
    $bw.Write([byte]0)
    $bw.Write([uint16]1)
    $bw.Write([uint16]32)
    $bw.Write([uint32]$e.Data.Length)
    $bw.Write([uint32]$offset)
    $offset += $e.Data.Length
}
foreach ($e in $entries) { $bw.Write($e.Data) }
$bw.Flush()
[System.IO.File]::WriteAllBytes($Out, $stream.ToArray())
$bw.Dispose()
$stream.Dispose()
$len = (Get-Item $Out).Length
Write-Host "wrote $Out ($len bytes, $($entries.Count) images: $($sizes -join '/'))"
