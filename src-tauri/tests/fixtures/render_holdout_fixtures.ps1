Add-Type -AssemblyName System.Drawing

$jsonPath = Join-Path $PSScriptRoot "holdout_benchmark.json"
$rawJson = [System.IO.File]::ReadAllText($jsonPath, [System.Text.Encoding]::UTF8)
$data = $rawJson | ConvertFrom-Json

# 1. Render technical_holdout_50.png as clean 2-column developer reference screen (1600x1400)
$techTokens = $data.technical_tokens
$half = [int][Math]::Ceiling($techTokens.Count / 2.0)
$col1 = $techTokens[0..($half - 1)] -join "`n"
$col2 = $techTokens[$half..($techTokens.Count - 1)] -join "`n"

$bmpTech = New-Object System.Drawing.Bitmap 1800, 1500
$gTech = [System.Drawing.Graphics]::FromImage($bmpTech)
$gTech.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$gTech.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$gTech.Clear([System.Drawing.Color]::White)
$fontTech = New-Object System.Drawing.Font("Segoe UI", 16, [System.Drawing.FontStyle]::Regular)
$brushTech = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(20, 20, 20))

$formatTech = New-Object System.Drawing.StringFormat
$formatTech.FormatFlags = [System.Drawing.StringFormatFlags]::NoWrap

for ($i = 0; $i -lt $half; $i++) {
    $y = 40.0 + ($i * 52.0)
    $gTech.DrawString($techTokens[$i], $fontTech, $brushTech, 60.0, $y, $formatTech)
}
for ($i = $half; $i -lt $techTokens.Count; $i++) {
    $y = 40.0 + (($i - $half) * 52.0)
    $gTech.DrawString($techTokens[$i], $fontTech, $brushTech, 920.0, $y, $formatTech)
}

$outTech1 = Join-Path $PSScriptRoot "technical_holdout_50.png"
$bmpTech.Save($outTech1, [System.Drawing.Imaging.ImageFormat]::Png)
$outTech2 = Join-Path $PSScriptRoot "../../../tests/fixtures"
if (Test-Path $outTech2) {
    $bmpTech.Save((Join-Path $outTech2 "technical_holdout_50.png"), [System.Drawing.Imaging.ImageFormat]::Png)
}

$formatTech.Dispose()
$brushTech.Dispose()
$fontTech.Dispose()
$gTech.Dispose()
$bmpTech.Dispose()
Write-Host "Generated technical_holdout_50.png in 2-column layout (1600x1400, Consolas 15pt, $($techTokens.Count) tokens)"

# 2. Render mixed_holdout.png
$mixedLines = $data.mixed_lines
$mixedText = $mixedLines -join "`n"
$bmpMixed = New-Object System.Drawing.Bitmap 900, 360
$gMixed = [System.Drawing.Graphics]::FromImage($bmpMixed)
$gMixed.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
$gMixed.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$gMixed.Clear([System.Drawing.Color]::White)
$fontMixed = New-Object System.Drawing.Font("Segoe UI", 16, [System.Drawing.FontStyle]::Regular)
$brushMixed = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::Black)
$rectMixed = New-Object System.Drawing.RectangleF 30.0, 30.0, 840.0, 300.0
$formatMixed = New-Object System.Drawing.StringFormat
$gMixed.DrawString($mixedText, $fontMixed, $brushMixed, $rectMixed, $formatMixed)

$outMixed1 = Join-Path $PSScriptRoot "mixed_holdout.png"
$bmpMixed.Save($outMixed1, [System.Drawing.Imaging.ImageFormat]::Png)
if (Test-Path $outTech2) {
    $bmpMixed.Save((Join-Path $outTech2 "mixed_holdout.png"), [System.Drawing.Imaging.ImageFormat]::Png)
}

$formatMixed.Dispose()
$brushMixed.Dispose()
$fontMixed.Dispose()
$gMixed.Dispose()
$bmpMixed.Dispose()
Write-Host "Generated mixed_holdout.png ($($mixedLines.Count) lines)"
