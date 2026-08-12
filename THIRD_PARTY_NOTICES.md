# 第三方依赖责任记录

本文件记录当前开发阶段的发布责任边界，不替代正式发布时生成的许可证清单。

当前版本没有复制 `reference-projects/` 中的源代码、字典数据或图标资源。桌面运行时使用 Tauri、WebView2、React、Vite 以及 Rust 生态依赖；具体版本由 `package-lock.json` 和 `src-tauri/Cargo.lock` 固定。

正式发布前需要完成以下核验：

1. 从 npm 依赖树和 Cargo 依赖树生成完整许可证及版权清单。
2. 核对 Tauri、WebView2、Windows 凭据组件和打包工具的再分发条件。
3. 将最终清单与安装包一起发布，并重新检查后续加入的字典、PDF 或上游项目代码是否引入新的许可证义务。

## open-dictionary 数据

Lilt 在词典缺失或用户从设置页手动更新时，从
[`ahpxex/open-dictionary`](https://github.com/ahpxex/open-dictionary) 的 GitHub
Release 获取 `distribution.sqlite.gz`。当前接入的工件为 v2.0，分发契约为
`distribution_entry_v5`，SQLite 打包契约为 `distribution_sqlite_v1`，约含
84,212 个英语词条。应用不把工件放入安装包，而是下载到应用数据目录的独立
词典缓存中；安装前必须校验同一 Release 的 `SHA256SUMS.txt`，并通过 SQLite
完整性检查。

该词典数据是 English Wiktionary 内容的衍生作品，按 Creative Commons
Attribution-ShareAlike 4.0 International（CC BY-SA 4.0）发布。源内容由
Wiktionary 贡献者提供，相关署名应保留；数据工件及其修改版本需要按相同许可
共享。上游数据许可和署名说明见
[`open-dictionary/LICENSE-DATA.md`](https://github.com/ahpxex/open-dictionary/blob/main/LICENSE-DATA.md)，
许可全文见
[`CC BY-SA 4.0`](https://creativecommons.org/licenses/by-sa/4.0/)。

词典设置页显示已安装 Release、词条数量、数据契约版本和本地占用空间，便于
定位数据版本与更新问题。
