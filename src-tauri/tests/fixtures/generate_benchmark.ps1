Add-Type -AssemblyName System.Drawing

$jsonPath = Join-Path $PSScriptRoot "benchmark_corpus.json"
$outDir = Join-Path $PSScriptRoot "vietnamese_benchmark"

if (!(Test-Path $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
}

$rawJson = [System.IO.File]::ReadAllText($jsonPath, [System.Text.Encoding]::UTF8)
$fixtures = $rawJson | ConvertFrom-Json

Write-Host "Found $($fixtures.Count) fixtures in corpus JSON. Rendering PNGs..."

foreach ($f in $fixtures) {
    $outPath = Join-Path $outDir $f.name
    $bmp = New-Object System.Drawing.Bitmap $f.width, $f.height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    
    $bgColor = [System.Drawing.ColorTranslator]::FromHtml($f.bg)
    $fgColor = [System.Drawing.ColorTranslator]::FromHtml($f.fg)
    $g.Clear($bgColor)
    
    $fontStyle = if ($f.isBold) { [System.Drawing.FontStyle]::Bold } else { [System.Drawing.FontStyle]::Regular }
    $fontObj = New-Object System.Drawing.Font($f.font, [float]$f.size, $fontStyle)
    $brushObj = New-Object System.Drawing.SolidBrush($fgColor)
    
    $rect = New-Object System.Drawing.RectangleF 20.0, 18.0, ($f.width - 40.0), ($f.height - 30.0)
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

Write-Host "All $($fixtures.Count) benchmark fixtures successfully generated in: $outDir"
