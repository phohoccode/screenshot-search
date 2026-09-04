Add-Type -AssemblyName System.Drawing
$items = @(
    @{ name = "alpha.png"; text = "ALPHA OCR UNIQUE 12345" },
    @{ name = "beta.png"; text = "BETA OCR UNIQUE 67890" },
    @{ name = "gamma.png"; text = "GAMMA OCR UNIQUE 54321" }
)

foreach ($item in $items) {
    $bmp = New-Object System.Drawing.Bitmap 600, 140
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $g.Clear([System.Drawing.Color]::White)
    $font = New-Object System.Drawing.Font("Segoe UI", 24, [System.Drawing.FontStyle]::Bold)
    $brush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::Black)
    $g.DrawString($item.text, $font, $brush, 30.0, 45.0)
    
    $p1 = Join-Path $PSScriptRoot $item.name
    $bmp.Save($p1, [System.Drawing.Imaging.ImageFormat]::Png)
    $p2 = Join-Path $PSScriptRoot "../../../tests/fixtures/data_integrity" $item.name
    if (Test-Path (Split-Path $p2)) {
        $bmp.Save($p2, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    
    $font.Dispose()
    $brush.Dispose()
    $g.Dispose()
    $bmp.Dispose()
}
Write-Host "Data integrity fixtures regenerated."
