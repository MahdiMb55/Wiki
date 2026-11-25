[CmdletBinding()]
param([switch]$DebugBuild)

$ErrorActionPreference = 'Stop'
$platform = 'x86_64-pc-windows-msvc'
$cache = Join-Path $PSScriptRoot 'target/wiki-downloads'
$unpacked = Join-Path $cache 'unpacked'
$bundle = Join-Path $PSScriptRoot 'target/wiki-app'
$modelName = 'vosk-model-small-en-us-0.15'
$runtimeName = 'vosk-win64-0.3.45'

function Get-VerifiedArchive {
    param([string]$Name, [string]$Uri, [string]$Sha256)

    $archive = Join-Path $cache "$Name.zip"
    if (-not (Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $Sha256) {
        $download = "$archive.download"
        Invoke-WebRequest -UseBasicParsing -Uri $Uri -OutFile $download
        if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $Sha256) {
            throw "Checksum verification failed for $Name."
        }
        Move-Item -LiteralPath $download -Destination $archive -Force
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $unpacked -Force
}

New-Item -Path $cache, $unpacked -ItemType Directory -Force | Out-Null
Get-VerifiedArchive -Name $runtimeName `
    -Uri "https://github.com/alphacep/vosk-api/releases/download/v0.3.45/$runtimeName.zip" `
    -Sha256 'F1DCC9CCA460630F81EA8F71794F69C80BED6556D2A4E6237B5785E1D2DFF34B'
Get-VerifiedArchive -Name $modelName `
    -Uri "https://alphacephei.com/vosk/models/$modelName.zip" `
    -Sha256 '30F26242C4EB449F948E42CB302DD7A686CB29A3423A8367F99FF41780942498'

$cargoArgs = @('build', '--locked', '--target', $platform)
$profile = 'debug'
if (-not $DebugBuild) {
    $cargoArgs += '--release'
    $profile = 'release'
}
Push-Location -LiteralPath $PSScriptRoot
try {
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        throw "Cargo build failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

$model = Join-Path $bundle 'model'
New-Item -Path $bundle, $model -ItemType Directory -Force | Out-Null
Get-ChildItem -LiteralPath (Join-Path $unpacked $modelName) | Copy-Item -Destination $model -Recurse -Force
Get-ChildItem -LiteralPath (Join-Path $unpacked $runtimeName) -Filter '*.dll' | Copy-Item -Destination $bundle -Force
$exe = Join-Path $bundle 'wiki.exe'
Copy-Item -LiteralPath (Join-Path $PSScriptRoot "target/$platform/$profile/wiki.exe") -Destination $exe -Force
Write-Output "Wiki is ready at $exe"
