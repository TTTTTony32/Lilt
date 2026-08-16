[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("amd64")]
    [string] $Architecture,

    [string] $DistributionVersion = "local",

    [string] $ProjectRoot = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $ProjectRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
}

$manifestPath = Join-Path $ProjectRoot "pdf-engine\manifests\windows-x86_64.json"
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$engineVersion = "babeldoc-$($manifest.babeldocVersion)"
$target = [string]$manifest.target
$buildRoot = Join-Path $ProjectRoot "pdf-engine\build\$target"
$runtimeRoot = Join-Path $buildRoot $engineVersion
$downloadRoot = Join-Path $buildRoot "downloads"
$pythonExtractRoot = Join-Path $buildRoot "python-extract"
$offlineRoot = Join-Path $buildRoot "offline-assets"
$distRoot = Join-Path $ProjectRoot "pdf-engine\dist"
$pythonArchive = Join-Path $downloadRoot "python-$($manifest.pythonVersion)-$Architecture.zip"
$archiveName = "$engineVersion-$target.zip"
$archivePath = Join-Path $distRoot $archiveName
$metadataPath = Join-Path $distRoot "engine-metadata-$target.json"

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory = $true)][string] $Path,
        [Parameter(Mandatory = $true)][string] $Content
    )

    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string] $FilePath,
        [Parameter(Mandatory = $true)][string[]] $ArgumentList,
        [Parameter(Mandatory = $true)][string] $Label
    )

    & $FilePath @ArgumentList
    if ($LASTEXITCODE -ne 0) {
        throw "$Label 失败，退出码：$LASTEXITCODE"
    }
}

function Download-Checked {
    param(
        [Parameter(Mandatory = $true)][string] $Uri,
        [Parameter(Mandatory = $true)][string] $Destination,
        [Parameter(Mandatory = $true)][string] $ExpectedSha256,
        [Parameter(Mandatory = $true)][string] $Label
    )

    if (-not (Test-Path -LiteralPath $Destination -PathType Leaf)) {
        Write-Host "下载 $Label：$Uri"
        Invoke-WebRequest -Uri $Uri -OutFile $Destination
    }
    $actual = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $ExpectedSha256.ToLowerInvariant()) {
        Remove-Item -LiteralPath $Destination -Force
        throw "$Label SHA-256 校验失败：期望 $ExpectedSha256，实际 $actual"
    }
}

function Set-EmbeddedPythonPath {
    param([Parameter(Mandatory = $true)][string] $PythonRoot)

    $pthFiles = @(Get-ChildItem -LiteralPath $PythonRoot -Filter "*._pth" -File)
    if ($pthFiles.Count -ne 1) {
        throw "Python embeddable package 的 _pth 文件数量异常：$($pthFiles.Count)"
    }
    $lines = @(Get-Content -LiteralPath $pthFiles[0].FullName)
    foreach ($requiredLine in @("Lib", "Lib/site-packages", "import site")) {
        if (-not ($lines -contains $requiredLine)) {
            $lines += $requiredLine
        }
    }
    Write-Utf8NoBom -Path $pthFiles[0].FullName -Content (($lines -join "`r`n") + "`r`n")
}

function Set-EngineHome {
    param([Parameter(Mandatory = $true)][string] $EngineHome)

    $drive = Split-Path -Qualifier $EngineHome
    $homePath = $EngineHome.Substring($drive.Length)
    $env:HOME = $EngineHome
    $env:USERPROFILE = $EngineHome
    $env:HOMEDRIVE = $drive
    $env:HOMEPATH = $homePath
    $env:XDG_CACHE_HOME = Join-Path $EngineHome ".cache"
    $env:TIKTOKEN_CACHE_DIR = Join-Path $EngineHome ".cache\babeldoc\tiktoken"
    $env:PYTHONNOUSERSITE = "1"
}

function Relative-EnginePath {
    param(
        [Parameter(Mandatory = $true)][string] $Root,
        [Parameter(Mandatory = $true)][string] $Path
    )

    $rootUri = [System.Uri]::new(([System.IO.Path]::GetFullPath($Root).TrimEnd("\") + "\"))
    $pathUri = [System.Uri]::new([System.IO.Path]::GetFullPath($Path))
    return $rootUri.MakeRelativeUri($pathUri).ToString().Replace("\", "/")
}

if (Test-Path -LiteralPath $buildRoot) {
    Remove-Item -LiteralPath $buildRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $runtimeRoot, $downloadRoot, $pythonExtractRoot, $offlineRoot, $distRoot -Force | Out-Null

$uv = Get-Command uv -ErrorAction Stop
$uvPath = if ($uv.Source) { $uv.Source } else { $uv.Path }

Download-Checked `
    -Uri $manifest.pythonUrl `
    -Destination $pythonArchive `
    -ExpectedSha256 $manifest.pythonSha256 `
    -Label "Python $($manifest.pythonVersion)"

Expand-Archive -LiteralPath $pythonArchive -DestinationPath $pythonExtractRoot -Force
$pythonFiles = Get-ChildItem -LiteralPath $pythonExtractRoot -Force
if ($pythonFiles.Count -eq 1 -and $pythonFiles[0].PSIsContainer) {
    $pythonSourceRoot = $pythonFiles[0].FullName
} else {
    $pythonSourceRoot = $pythonExtractRoot
}
$pythonRoot = Join-Path $runtimeRoot "python"
New-Item -ItemType Directory -Path $pythonRoot -Force | Out-Null
Copy-Item -Path (Join-Path $pythonSourceRoot "*") -Destination $pythonRoot -Recurse -Force
$pythonPath = Join-Path $pythonRoot "python.exe"
if (-not (Test-Path -LiteralPath $pythonPath -PathType Leaf)) {
    throw "Python embeddable package 缺少 python.exe"
}
Set-EmbeddedPythonPath -PythonRoot $pythonRoot

$sitePackages = Join-Path $pythonRoot "Lib\site-packages"
New-Item -ItemType Directory -Path $sitePackages -Force | Out-Null
Invoke-Checked -FilePath $uvPath -ArgumentList @(
    "pip", "install",
    "--target", $sitePackages,
    "--python-version", $manifest.pythonVersionMajorMinor,
    "--python-platform", $manifest.uvPlatform,
    "--index-url", "https://pypi.org/simple",
    "BabelDOC==$($manifest.babeldocVersion)"
) -Label "安装 BabelDOC $($manifest.babeldocVersion) 及依赖"

$workerDestination = Join-Path $runtimeRoot "pdf-worker\worker.py"
New-Item -ItemType Directory -Path (Split-Path -Parent $workerDestination) -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $ProjectRoot "src-tauri\python_worker\worker.py") -Destination $workerDestination -Force

$licenseRoot = Join-Path $runtimeRoot "licenses"
New-Item -ItemType Directory -Path $licenseRoot -Force | Out-Null
Invoke-WebRequest -Uri $manifest.babeldocLicenseUrl -OutFile (Join-Path $licenseRoot "BabelDOC-AGPL-3.0.txt")
$pythonLicense = Join-Path $pythonRoot "LICENSE.txt"
if (-not (Test-Path -LiteralPath $pythonLicense -PathType Leaf)) {
    throw "Python embeddable package 缺少 LICENSE.txt"
}
Copy-Item -LiteralPath $pythonLicense -Destination (Join-Path $licenseRoot "Python-PSF.txt") -Force

$packageNames = @(Get-ChildItem -LiteralPath $sitePackages -Filter "*.dist-info" -Directory | ForEach-Object {
    $metadata = Join-Path $_.FullName "METADATA"
    if (Test-Path -LiteralPath $metadata) {
        $name = Select-String -LiteralPath $metadata -Pattern "^Name:\s*(.+)$" | Select-Object -First 1
        $version = Select-String -LiteralPath $metadata -Pattern "^Version:\s*(.+)$" | Select-Object -First 1
        if ($name -and $version) {
            "$($name.Matches[0].Groups[1].Value) $($version.Matches[0].Groups[1].Value)"
        }
    }
}) | Sort-Object
Write-Utf8NoBom `
    -Path (Join-Path $licenseRoot "THIRD-PARTY-SOURCES.txt") `
    -Content ((@(
        "BabelDOC runtime dependency inventory generated by Lilt.",
        "Licenses remain governed by each package and its upstream project.",
        ""
    ) + $packageNames) -join "`r`n")

Set-EngineHome -EngineHome $runtimeRoot
Push-Location $runtimeRoot
try {
    Invoke-Checked -FilePath $pythonPath -ArgumentList @(
        "-s", "-c", "import babeldoc; assert babeldoc.__version__ == '$($manifest.babeldocVersion)'"
    ) -Label "验证 BabelDOC Python 导入"

    Invoke-Checked -FilePath $pythonPath -ArgumentList @(
        "-s", "-m", "babeldoc.main", "--generate-offline-assets", $offlineRoot
    ) -Label "生成 BabelDOC 离线资源"
    $offlinePackages = @(Get-ChildItem -LiteralPath $offlineRoot -Filter "*.zip" -File)
    if ($offlinePackages.Count -ne 1) {
        throw "BabelDOC 离线资源包数量异常：$($offlinePackages.Count)"
    }
    Invoke-Checked -FilePath $pythonPath -ArgumentList @(
        "-s", "-m", "babeldoc.main", "--restore-offline-assets", $offlinePackages[0].FullName
    ) -Label "恢复 BabelDOC 离线资源"
} finally {
    Pop-Location
}

$resourceFiles = @(Get-ChildItem -LiteralPath $runtimeRoot -File -Recurse | Where-Object {
    $_.FullName -ne (Join-Path $runtimeRoot "runtime.json")
})
if ($resourceFiles.Count -eq 0) {
    throw "PDF Engine 没有生成资源文件"
}
foreach ($directory in @(Get-ChildItem -LiteralPath $runtimeRoot -Directory -Recurse)) {
    if ($directory.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        throw "PDF Engine 不允许符号链接或重解析目录：$($directory.FullName)"
    }
}

$resourceSize = [int64](($resourceFiles | Measure-Object -Property Length -Sum).Sum)
$resourceCount = $resourceFiles.Count
$criticalResourcePaths = @(
    "python/python.exe",
    "python/python313.dll",
    "python/python313.zip",
    "python/python313._pth",
    "pdf-worker/worker.py",
    "licenses/BabelDOC-AGPL-3.0.txt",
    "licenses/Python-PSF.txt",
    "licenses/THIRD-PARTY-SOURCES.txt"
)
$resources = @($criticalResourcePaths | ForEach-Object {
    $criticalPath = Join-Path $runtimeRoot $_.Replace("/", "\")
    if (-not (Test-Path -LiteralPath $criticalPath -PathType Leaf)) {
        throw "PDF Engine 缺少关键资源：$_"
    }
    $criticalFile = Get-Item -LiteralPath $criticalPath
    if ($criticalFile.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
        throw "PDF Engine 不允许符号链接或重解析文件：$criticalPath"
    }
    [PSCustomObject]@{
        path = $_
        sha256 = (Get-FileHash -LiteralPath $criticalPath -Algorithm SHA256).Hash.ToLowerInvariant()
        size = [int64]$criticalFile.Length
        required = $true
    }
})

$runtimeManifest = [PSCustomObject]@{
    engine_version = $engineVersion
    target = $target
    python = "python/python.exe"
    worker = "pdf-worker/worker.py"
    python_version = [string]$manifest.pythonVersion
    babeldoc_version = [string]$manifest.babeldocVersion
    pdfmathtranslate_revision = [string]$manifest.pdfmathtranslateRevision
    distribution_version = $DistributionVersion
    resource_count = $resourceCount
    resource_size_bytes = $resourceSize
    resources = $resources
    licenses = @(
        [PSCustomObject]@{
            name = "BabelDOC"
            license = "AGPL-3.0"
            source = [string]$manifest.babeldocLicenseUrl
            files = @("licenses/BabelDOC-AGPL-3.0.txt")
        },
        [PSCustomObject]@{
            name = "Python"
            license = "PSF License"
            source = "https://www.python.org/downloads/release/python-$($manifest.pythonVersion.Replace('.', ''))/"
            files = @("licenses/Python-PSF.txt")
        },
        [PSCustomObject]@{
            name = "BabelDOC dependencies"
            license = "See each package"
            source = "https://pypi.org/"
            files = @("licenses/THIRD-PARTY-SOURCES.txt")
        }
    )
}
Write-Utf8NoBom `
    -Path (Join-Path $runtimeRoot "runtime.json") `
    -Content (($runtimeManifest | ConvertTo-Json -Depth 10) + "`r`n")

if (Test-Path -LiteralPath $archivePath) {
    Remove-Item -LiteralPath $archivePath -Force
}
Compress-Archive -LiteralPath $runtimeRoot -DestinationPath $archivePath -CompressionLevel Optimal
$archiveHash = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
$manifestHash = (Get-FileHash -LiteralPath (Join-Path $runtimeRoot "runtime.json") -Algorithm SHA256).Hash.ToLowerInvariant()
$archiveSize = (Get-Item -LiteralPath $archivePath).Length

$metadata = [PSCustomObject]@{
    schemaVersion = 1
    engineVersion = $engineVersion
    distributionVersion = $DistributionVersion
    target = $target
    zipName = $archiveName
    sha256 = $archiveHash
    size = [int64]$archiveSize
    manifestSha256 = $manifestHash
    pythonVersion = [string]$manifest.pythonVersion
    babeldocVersion = [string]$manifest.babeldocVersion
    resourceSize = $resourceSize
}
Write-Utf8NoBom -Path $metadataPath -Content (($metadata | ConvertTo-Json -Depth 5) + "`r`n")

Write-Host "PDF Engine 已生成：$archivePath"
Write-Host "压缩包大小：$archiveSize 字节"
Write-Host "资源体积：$($metadata.resourceSize) 字节"
