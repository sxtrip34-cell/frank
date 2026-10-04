<#
  Frank - voice setup.

  Downloads the free, open-source tools Frank uses to hear and speak, into
  %LOCALAPPDATA%\Frank\voice. Everything runs on your own computer afterwards:
  no audio ever leaves it.

    whisper.cpp  (MIT)            speech -> text
    Whisper model (MIT)           large-v3-turbo with an NVIDIA GPU, small otherwise
    Piper        (MIT)            text -> speech
    Voices: en_US-joe (CC0), ru_RU-dmitri (CC0), tr_TR-dfki (CC BY-NC-SA 4.0)

  Every download is pinned to one release and checked against its SHA-256
  before it is used: a file that was changed on the way, or at the source, is
  deleted instead of installed.

  The installer runs this for you when you say yes to the voice tools. By hand
  (PowerShell):
    powershell -ExecutionPolicy Bypass -File scripts\setup-voice.ps1
    ... -Cpu          use the CPU build even if an NVIDIA GPU is present
    ... -NoTurkish    skip the Turkish voice (its licence is non-commercial)
    ... -Pause        wait for Enter before closing the window

  Run it again at any time: files already in place are kept.
#>
param(
  [switch]$Cpu,
  [switch]$NoTurkish,
  [switch]$Pause
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

# Pinned sources: a release tag on GitHub, a commit on Hugging Face.
$WhisperRelease = "https://github.com/ggml-org/whisper.cpp/releases/download/b5130"
$Models_Url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1"
$PiperRelease = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2"
$Voices_Url = "https://huggingface.co/rhasspy/piper-voices/resolve/c10ece1aade47bb51c153c893d14e5bf8e5b7117"

# The SHA-256 of every file this script downloads.
$Sha256 = @{
  "whisper-bin-x64.zip"               = "f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c"
  "whisper-cublas-12.4.0-bin-x64.zip" = "af520ddd034d985b55dfeea3e465ed93653ba2aee1a55e865033edc548c272a7"
  "ggml-small.bin"                    = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
  "ggml-large-v3-turbo-q5_0.bin"      = "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2"
  "piper_windows_amd64.zip"           = "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea"
  "en_US-joe-medium.onnx"             = "58afce0321b8d9c46d7cdf9c16500cc55a793b4220212dba6b70fb788b3baf06"
  "en_US-joe-medium.onnx.json"        = "3d6d5410b3795cb1950595247ef8f06190719e6fdbfa3a2356d8ec368e1aad33"
  "ru_RU-dmitri-medium.onnx"          = "f073356ebc4bd0f80c5af58df2953a5988bd5bdab1eb38635ce960b071fbefcb"
  "ru_RU-dmitri-medium.onnx.json"     = "667ef3117bc642c2892dff7690d8bdc8ca4228aeaa783b2dc1416df632855e0d"
  "tr_TR-dfki-medium.onnx"            = "2844717f524ab965d3fe86e60562cbb601d3e456836efcc2196cc3a14112a8fb"
  "tr_TR-dfki-medium.onnx.json"       = "13ebd7810f1b61b5027583cf3131a0a233b6ea81c38f2200ebc4ff41c3cca039"
}

$Voice = Join-Path $env:LOCALAPPDATA "Frank\voice"
$Whisper = Join-Path $Voice "whisper"
$Models = Join-Path $Voice "models"
$PiperDir = Join-Path $Voice "piper"
$Voices = Join-Path $Voice "piper-voices"
$Downloads = Join-Path $Voice "downloads"

function Test-Sha256([string]$Path, [string]$Name) {
  # -eq compares strings case-insensitively; Get-FileHash prints upper case.
  return (Get-FileHash -Algorithm SHA256 $Path).Hash -eq $Sha256[$Name]
}

function Get-File([string]$Url, [string]$Path) {
  $name = Split-Path $Path -Leaf
  if ((Test-Path $Path) -and (Test-Sha256 $Path $name)) {
    Write-Host "  already here: $name"
    return
  }
  Write-Host "  downloading $name ..."
  $partial = "$Path.partial"
  # curl.exe ships with Windows 10/11 and resumes and retries better than Invoke-WebRequest.
  & curl.exe -L --fail --retry 5 --retry-all-errors -C - -o $partial $Url
  if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
  if (-not (Test-Sha256 $partial $name)) {
    Remove-Item -Force $partial
    throw "$name does not match its known SHA-256 checksum, so it was deleted and not installed."
  }
  Move-Item -Force $partial $Path
}

$failed = $false
try {
  foreach ($d in @($Voice, $Whisper, $Models, $Voices, $Downloads)) {
    New-Item -ItemType Directory -Force -Path $d | Out-Null
  }

  $gpu = -not $Cpu -and [bool](Get-CimInstance Win32_VideoController | Where-Object { $_.Name -match "NVIDIA" })

  Write-Host ""
  Write-Host "1/3  whisper.cpp ($(if ($gpu) { 'NVIDIA GPU build' } else { 'CPU build' }))"
  $zipName = if ($gpu) { "whisper-cublas-12.4.0-bin-x64.zip" } else { "whisper-bin-x64.zip" }
  $zip = Join-Path $Downloads $zipName
  if (-not (Test-Path (Join-Path $Whisper "whisper-server.exe"))) {
    Get-File "$WhisperRelease/$zipName" $zip
    $unpacked = Join-Path $Downloads "whisper-unpacked"
    Expand-Archive -Force $zip $unpacked
    $release = Join-Path $unpacked "Release"
    Copy-Item (Join-Path $release "*.dll") $Whisper -Force
    Copy-Item (Join-Path $release "whisper-cli.exe") $Whisper -Force
    Copy-Item (Join-Path $release "whisper-server.exe") $Whisper -Force
    Remove-Item -Recurse -Force $unpacked
  } else {
    Write-Host "  already here: whisper"
  }

  Write-Host ""
  Write-Host "2/3  Whisper model"
  if ($gpu) {
    # Best with mixed-language speech; needs about 1 GB of graphics memory.
    Get-File "$Models_Url/ggml-large-v3-turbo-q5_0.bin" (Join-Path $Models "ggml-large-v3-turbo-q5_0.bin")
  } else {
    # Fast enough on a CPU; a little less accurate.
    Get-File "$Models_Url/ggml-small.bin" (Join-Path $Models "ggml-small.bin")
  }

  Write-Host ""
  Write-Host "3/3  Piper and voices"
  if (-not (Test-Path (Join-Path $PiperDir "piper.exe"))) {
    $piperZip = Join-Path $Downloads "piper_windows_amd64.zip"
    Get-File "$PiperRelease/piper_windows_amd64.zip" $piperZip
    Expand-Archive -Force $piperZip $Voice   # unpacks to ...\voice\piper
  } else {
    Write-Host "  already here: piper"
  }

  $list = @(
    @{ Name = "en_US-joe-medium"; Path = "en/en_US/joe/medium" },
    @{ Name = "ru_RU-dmitri-medium"; Path = "ru/ru_RU/dmitri/medium" }
  )
  if (-not $NoTurkish) {
    $list += @{ Name = "tr_TR-dfki-medium"; Path = "tr/tr_TR/dfki/medium" }
  }
  foreach ($v in $list) {
    Get-File "$Voices_Url/$($v.Path)/$($v.Name).onnx" (Join-Path $Voices "$($v.Name).onnx")
    Get-File "$Voices_Url/$($v.Path)/$($v.Name).onnx.json" (Join-Path $Voices "$($v.Name).onnx.json")
  }

  Remove-Item -Recurse -Force $Downloads -ErrorAction SilentlyContinue

  Write-Host ""
  Write-Host "Done. Frank's voice tools are in $Voice"
  if (-not $NoTurkish) {
    Write-Host ""
    Write-Host "Note: the Turkish voice (tr_TR-dfki) is licensed CC BY-NC-SA 4.0 -"
    Write-Host "free for personal use, not for commercial use. The English and Russian"
    Write-Host "voices are CC0. Run with -NoTurkish to leave it out."
  }
} catch {
  $failed = $true
  Write-Host ""
  Write-Host "Voice setup stopped: $($_.Exception.Message)" -ForegroundColor Red
  Write-Host "Run it again to carry on: finished downloads are kept."
}

if ($Pause) {
  Write-Host ""
  Read-Host "Press Enter to close" | Out-Null
}
if ($failed) { exit 1 }
