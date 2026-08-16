# Engine 许可证与来源

Engine 构建脚本会把以下许可证文本复制到每个架构的发布包中，并在 `runtime.json` 中记录文件路径与来源：

- BabelDOC 0.6.4：AGPL-3.0，来源为 BabelDOC v0.6.4 的 `LICENSE` 文件。
- Python 3.13.15：PSF License，来源为 CPython v3.13.15 的 `LICENSE.txt` 文件。
- BabelDOC 依赖：来自 PyPI 的运行时依赖。构建过程会把已解析的发行包名称和版本写入 `THIRD-PARTY-SOURCES.txt`；各依赖的许可证仍以其发行包附带文本和上游项目声明为准。
- 离线字体、模型、CMap 与 OCR 资源：由 BabelDOC 0.6.4 的离线资源命令生成，具体来源和许可证以资源包内的元数据及 BabelDOC 上游声明为准。

Engine 资源由独立的 PDF Engine Release 托管，不写入 NSIS 安装包。再分发时应同时保留这些许可证文本和资源索引中的来源信息。
