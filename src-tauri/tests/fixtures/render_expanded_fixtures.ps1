Add-Type -AssemblyName System.Drawing

$jsonPath = Join-Path $PSScriptRoot "expanded_benchmark.json"
$rawJson = [System.IO.File]::ReadAllText($jsonPath, [System.Text.Encoding]::UTF8)
$fixtures = $rawJson | ConvertFrom-Json

foreach ($f in $fixtures) {
    $outPath = Join-Path $PSScriptRoot $f.name
    $bmp = New-Object System.Drawing.Bitmap $f.width, $f.height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.Clear([System.Drawing.Color]::White)
    
    $fontObj = New-Object System.Drawing.Font($f.font, [float]$f.size, [System.Drawing.FontStyle]::Regular)
    $brushObj = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(20, 20, 20))
    
    $rect = New-Object System.Drawing.RectangleF 30.0, 25.0, ($f.width - 60.0), ($f.height - 50.0)
    $format = New-Object System.Drawing.StringFormat
    $format.Alignment = [System.Drawing.StringAlignment]::Near
    $format.LineAlignment = [System.Drawing.StringAlignment]::Near
    
    $g.DrawString($f.text, $fontObj, $brushObj, $rect, $format)
    
    $bmp.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)
    
    $format.Dispose()
    $brushObj.Dispose()
    $fontObj.Dispose()
    $g.Dispose()
    $bmp.Dispose()
    
    Write-Host "Generated: $($f.name) ($($f.width)x$($f.height))"
}
Write-Host "All expanded benchmark fixtures rendered successfully from JSON."
