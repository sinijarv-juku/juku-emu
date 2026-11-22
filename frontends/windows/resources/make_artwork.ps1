# Makes the Windows app's artwork from the Mac app's icon
# (frontends/macos/resources/Juku.icon), so both apps have one look (spec
# sections 11.1 and 11.8). Run it after the Mac icon changes, and commit what
# it writes:
#
#   powershell -ExecutionPolicy Bypass -File frontends\windows\resources\make_artwork.ps1
#
#   resources\Juku.ico                     the app's icon, 16 to 256 px
#   installer\WizardSmallImage-<n>.png     the installer's icon at the top right of its pages
#   installer\WizardImage-<w>x<h>.png      the installer's side panel on its last page
#
# It uses only what Windows 11 has: Microsoft Edge, run headless, draws the
# Icon Composer bundle's gradient and its two SVG layers, with a transparent
# background, and the script packs the icon's PNG images into an .ico file.
# macOS masks the icon itself; here the rounded square is drawn, a little
# inside the edges, as Windows icons are. The installer's images come in
# several sizes, from which Setup takes the one that best fits the screen's
# scale: its small image area is square, 58 px at 100%, and its large one
# 202 x 386 px at 100%.

$ErrorActionPreference = "Stop"
$windows = Split-Path $PSScriptRoot
$assets = (Resolve-Path "$windows\..\macos\resources\Juku.icon\Assets").Path -replace '\\', '/'
$edge = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
$work = Join-Path ([IO.Path]::GetTempPath()) "juku-artwork-$PID"
New-Item -ItemType Directory -Force $work | Out-Null

# The gradient of icon.json: #1f1f1f at the top to #0f0f0f at the bottom.
$gradient = "linear-gradient(to bottom, rgb(31, 31, 31), rgb(15, 15, 15))"
$layers = "<img src=`"file:///$assets/led.svg`"><img src=`"file:///$assets/label.svg`">"
$style = "html, body { margin: 0; padding: 0; background: transparent; overflow: hidden }" +
         " .layers img { position: absolute; left: 0; top: 0; width: 100%; height: 100% }"

# Draws `body` in a page of `width` x `height` px and returns the PNG.
function Render([int]$width, [int]$height, [string]$body) {
    $name = "$($width)x$height-$([guid]::NewGuid().ToString('N'))"
    $html = Join-Path $work "$name.html"
    $png = Join-Path $work "$name.png"
    Set-Content -Encoding UTF8 $html "<!doctype html><html><head><style>$style body { width: ${width}px; height: ${height}px }</style></head><body>$body</body></html>"
    $arguments = "--headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --default-background-color=00000000 " +
                 "--window-size=$width,$height --screenshot=`"$png`" `"file:///$($html -replace '\\', '/')`""
    Start-Process -FilePath $edge -ArgumentList $arguments -Wait -WindowStyle Hidden
    if (!(Test-Path $png)) { throw "Edge did not draw $name" }
    , [IO.File]::ReadAllBytes($png)
}

# The icon: the rounded square with 20% corners, 1/32 of the size inside the
# edges.
function Icon([int]$size) {
    $margin = $size / 32
    $side = $size - 2 * $margin
    Render $size $size ("<div class=`"layers`" style=`"position: absolute; left: ${margin}px; top: ${margin}px; width: ${side}px; " +
                        "height: ${side}px; border-radius: 20%; overflow: hidden; background: $gradient`">$layers</div>")
}

# The side panel: the icon's gradient over the whole panel, and its layers
# across the panel's width, the lettering a little above the middle.
function Panel([int]$width, [int]$height) {
    $top = [math]::Round(0.42 * $height - 0.58 * $width)
    Render $width $height ("<div style=`"position: absolute; left: 0; top: 0; width: ${width}px; height: ${height}px; background: $gradient`"></div>" +
                           "<div class=`"layers`" style=`"position: absolute; left: 0; top: ${top}px; width: ${width}px; height: ${width}px`">$layers</div>")
}

# ---- Juku.ico: ICONDIR, an ICONDIRENTRY per image, then the images (PNG,
# which Windows reads at every size).
$sizes = 16, 20, 24, 32, 40, 48, 64, 96, 128, 256
$images = foreach ($size in $sizes) { , (Icon $size) }
$out = New-Object IO.MemoryStream
$w = New-Object IO.BinaryWriter $out
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count

for ($i = 0; $i -lt $sizes.Count; $i++) {
    $size = $sizes[$i]
    $w.Write([byte]($size % 256)); $w.Write([byte]($size % 256))    # 0 means 256
    $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$images[$i].Length); $w.Write([uint32]$offset)
    $offset += $images[$i].Length
}

foreach ($image in $images) { $w.Write($image) }
$w.Flush()
[IO.File]::WriteAllBytes("$PSScriptRoot\Juku.ico", $out.ToArray())
"Wrote resources\Juku.ico: $($sizes -join ', ') px"

# ---- The installer's images, at 100%, 125%, 150%, 200% and 250%.
$installer = "$windows\installer"
Get-ChildItem $installer -Filter "Wizard*Image-*.png" | Remove-Item

foreach ($size in 58, 72, 87, 116, 145) {
    [IO.File]::WriteAllBytes("$installer\WizardSmallImage-$size.png", (Icon $size))
}

foreach ($scale in 1.0, 1.25, 1.5, 2.0, 2.5) {
    $width = [math]::Round(202 * $scale)
    $height = [math]::Round(386 * $scale)
    [IO.File]::WriteAllBytes("$installer\WizardImage-$($width)x$height.png", (Panel $width $height))
}

"Wrote installer\WizardSmallImage-*.png and installer\WizardImage-*.png"
Remove-Item -Recurse -Force $work
