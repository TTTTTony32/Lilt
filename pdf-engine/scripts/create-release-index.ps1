[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string] $AssetsDirectory,
    [Parameter(Mandatory = $true)][string] $ReleaseTag,
    [Parameter(Mandatory = $true)][string] $OutputPath
)

$ErrorActionPreference = "Stop"
if ($ReleaseTag -notmatch '^lilt-pdf-engine-babeldoc-0\.6\.4-r[1-9][0-9]*$') {
    throw "Engine Release 标签格式无效：$ReleaseTag，应使用 lilt-pdf-engine-babeldoc-0.6.4-rN"
}

$metadataFiles = @(Get-ChildItem -LiteralPath $AssetsDirectory -Filter "engine-metadata-*.json" -File)
if ($metadataFiles.Count -ne 1) {
    throw "期望一个 Windows x64 PDF Engine 元数据文件，实际找到 $($metadataFiles.Count) 个"
}

$metadata = @($metadataFiles | ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json })
$engineVersions = @($metadata.engineVersion | Select-Object -Unique)
$distributionVersions = @($metadata.distributionVersion | Select-Object -Unique)
if ($engineVersions.Count -ne 1 -or $engineVersions[0] -ne "babeldoc-0.6.4") {
    throw "Engine 版本不一致"
}
if ($distributionVersions.Count -ne 1) {
    throw "Engine 分发修订号不一致"
}
if ([string]$distributionVersions[0] -notlike "$ReleaseTag-*") {
    throw "Engine 分发修订号与 Release 标签不一致"
}

$assets = [ordered]@{}
foreach ($item in $metadata) {
    if ($item.target -ne "windows-x86_64") {
        throw "不支持的 Engine 架构：$($item.target)"
    }
    $zipPath = Join-Path $AssetsDirectory $item.zipName
    if (-not (Test-Path -LiteralPath $zipPath -PathType Leaf)) {
        throw "缺少 Engine ZIP：$($item.zipName)"
    }
    if ([int64]$item.size -le 0) {
        throw "Engine ZIP 大小无效：$($item.zipName)"
    }
    $actualSize = (Get-Item -LiteralPath $zipPath).Length
    if ([int64]$actualSize -ne [int64]$item.size) {
        throw "Engine ZIP 大小与元数据不一致：$($item.zipName)"
    }
    $actualHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualHash -ne $item.sha256.ToLowerInvariant()) {
        throw "Engine ZIP 摘要与元数据不一致：$($item.zipName)"
    }
    $assets[$item.target] = [ordered]@{
        url = "https://github.com/TTTTTony32/Lilt/releases/download/$ReleaseTag/$($item.zipName)"
        sha256 = $item.sha256.ToLowerInvariant()
        size = [int64]$item.size
        manifestSha256 = $item.manifestSha256.ToLowerInvariant()
    }
}
if (-not $assets.Contains("windows-x86_64")) {
    throw "Release 索引必须包含 Windows x64 Engine"
}

$index = [ordered]@{
    schemaVersion = 1
    engineVersion = "babeldoc-0.6.4"
    distributionVersion = $distributionVersions[0]
    assets = $assets
}
$encoding = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText($OutputPath, (($index | ConvertTo-Json -Depth 6) + "`r`n"), $encoding)
Write-Host "已生成 PDF Engine 资源索引"
