# PDF Engine 构建目录

这里保存 PDF Engine 的构建输入和发布脚本。Engine 不进入 Lilt 安装包，GitHub Actions 为 Windows x64 与 ARM64 分别生成一个自包含 ZIP，并将 ZIP 与资源索引上传到同一 Release。

构建结果位于 `build/` 和 `dist/`，这两个目录只保存 CI 或本地构建产物，已加入 `.gitignore`。正式运行时，Lilt 将 Engine 解压到：

```text
%APPDATA%/com.lilt.desktop/engines/pdf/babeldoc-0.6.4/
```

## 固定输入

- Python 3.13.15 embeddable package。x64 使用 `amd64` 包，ARM64 使用 `arm64` 包。
- BabelDOC 0.6.4，依赖从 PyPI 按固定版本解析，构建时写入资源清单。
- `src-tauri/python_worker/worker.py` 是 Worker 的唯一源码来源。
- BabelDOC 的离线资源在构建阶段生成并恢复到 Engine 私有缓存目录，正式运行不访问 PyPI、Git 仓库或系统 Python。

## 本地构建

需要 Windows PowerShell 7、uv，以及可访问 Python Package Index 和 BabelDOC 资源的网络环境：

```powershell
uv --version
pwsh -File .\pdf-engine\scripts\build-engine.ps1 -Architecture amd64 -DistributionVersion local
```

正式构建只允许使用 `amd64` 或 `arm64`。ARM64 构建应在 Windows ARM64 runner 上执行，以便验证目标架构的原生依赖与 Python 运行时。

构建脚本会下载 Python、安装 BabelDOC 及其运行依赖，生成离线资源，写入 `runtime.json`，完成 Python 导入检查，最后生成：

```text
pdf-engine/dist/babeldoc-0.6.4-windows-x86_64.zip
pdf-engine/dist/engine-metadata-windows-x86_64.json
```

元数据文件供发布作业生成 `pdf-engine-index.json`，其中包含 ZIP 摘要、大小和 `runtime.json` 摘要。资源索引不写入源码仓库。

## 发布约束

Engine ZIP 必须包含唯一的 `babeldoc-0.6.4/` 根目录。`runtime.json` 使用相对路径，记录完整文件数量、资源总占用，并对 Python、Worker、许可证等关键文件记录 SHA-256 与体积。客户端先校验 Release 索引和 ZIP，再解压、校验 manifest 与 Python 导入，最后才切换应用数据目录中的版本目录。完整 ZIP 摘要负责保护其余依赖文件，运行时不重复遍历整个 Engine。
