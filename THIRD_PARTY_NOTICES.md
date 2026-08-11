# 第三方依赖责任记录

本文件记录当前开发阶段的发布责任边界，不替代正式发布时生成的许可证清单。

当前版本没有复制 `reference-projects/` 中的源代码、字典数据或图标资源。桌面运行时使用 Tauri、WebView2、React、Vite 以及 Rust 生态依赖；具体版本由 `package-lock.json` 和 `src-tauri/Cargo.lock` 固定。

正式发布前需要完成以下核验：

1. 从 npm 依赖树和 Cargo 依赖树生成完整许可证及版权清单。
2. 核对 Tauri、WebView2、Windows 凭据组件和打包工具的再分发条件。
3. 将最终清单与安装包一起发布，并重新检查后续加入的字典、PDF 或上游项目代码是否引入新的许可证义务。
