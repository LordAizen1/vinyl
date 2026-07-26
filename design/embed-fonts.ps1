# Embeds the two woff2 files into design/turntable.html as data URIs.
#
# Chrome blocks fonts loaded over file:// as cross-origin, which would break the
# "opens standalone in a browser with zero build step" requirement in PLAN.md.
# The shipped app in src/ links the same files normally and does not need this.
#
# Idempotent: rewrites both @font-face blocks wholesale, so re-running after a
# font update is safe.
#
#   powershell -File design/embed-fonts.ps1

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$page = Join-Path $root "design\turntable.html"
$fonts = Join-Path $root "src\assets\fonts"

function ConvertTo-DataUri([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    "data:font/woff2;base64," + [System.Convert]::ToBase64String($bytes)
}

$archivo = ConvertTo-DataUri (Join-Path $fonts "ArchivoNarrow-latin.woff2")
$jetbrains = ConvertTo-DataUri (Join-Path $fonts "JetBrainsMono-latin.woff2")

$archivoBlock = @"
@font-face {
        font-family: "Archivo Narrow";
        font-style: normal;
        font-weight: 400 700;
        font-display: block;
        src: url("$archivo") format("woff2");
      }
"@

$jetbrainsBlock = @"
@font-face {
        font-family: "JetBrains Mono";
        font-style: normal;
        font-weight: 400;
        font-display: block;
        src: url("$jetbrains") format("woff2");
      }
"@

$html = [System.IO.File]::ReadAllText($page)

$archivoPattern = '@font-face \{\s*font-family: "Archivo Narrow";[\s\S]*?\n      \}'
$jetbrainsPattern = '@font-face \{\s*font-family: "JetBrains Mono";[\s\S]*?\n      \}'

if ($html -notmatch $archivoPattern) { throw "Archivo Narrow @font-face block not found" }
if ($html -notmatch $jetbrainsPattern) { throw "JetBrains Mono @font-face block not found" }

$html = [regex]::Replace($html, $archivoPattern, { $archivoBlock }, 1)
$html = [regex]::Replace($html, $jetbrainsPattern, { $jetbrainsBlock }, 1)

[System.IO.File]::WriteAllText($page, $html)

$size = [math]::Round((Get-Item $page).Length / 1KB, 1)
Write-Output "embedded both fonts; turntable.html is now $size KB"
