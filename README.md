# Lilt

Lilt 是面向中文用户的桌面阅读辅助工具。

## 主要功能

### 段落翻译

- 支持英语、简体中文、繁体中文、日语和韩语
- 基于LLM服务进行翻译，目前暂时仅支持 `OpenAI Completions API`
- 提示词支持内置 Prompt 和自定义 Prompt

### 词典

- 使用 [ahpxex/open-dictionary](https://github.com/ahpxex/open-dictionary) 词典库
- 支持收藏与个人词典导出（暂未做完）功能
- 查询词若曾在段落翻译中翻译过，所在句将自动作为例句出现

### 划词翻译

- 支持快捷键和自动触发两种模式
- 单词查询和段落翻译会自动路由到对应功能，结果显示在悬浮窗中

### 其他功能

- 术语表：支持手动添加原文-译文对和备注，支持导入（暂未做完）
- PDF全文翻译（基于 [PDFMathTranslate-Next](https://github.com/PDFMathTranslate-next/PDFMathTranslate-next) ）（暂未做完）

## 技术架构

- 前端：React、TypeScript、Vite
- 桌面运行时：Tauri 2
- 后端：Rust
- 数据库：SQLite

## 开发环境

- Node.js 20 或更高版本
- Rust stable MSVC 工具链
- Windows WebView2 Runtime
- Windows 10 或更高版本

## 开发与构建

安装前端依赖：

```bash
npm install
```

启动完整开发版：

```bash
npm run tauri -- dev
```

开发版的后端日志会输出到启动命令的终端。需要更详细的 Tauri 日志时，可以使用：

```bash
npm run tauri -- dev -v
```

只启动前端开发服务器：

```bash
npm run dev
```

质量检查：

```bash
npm run typecheck
npm run lint
npm test
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

构建前端和桌面安装包：

```bash
npm run build
npm run tauri -- build
```

## GitHub Actions Release

Release 工作流位于 `.github/workflows/release.yml`，只构建 Windows NSIS 安装包，分别生成 `amd64` 和 `arm64` 两个架构版本。

### 手动验证构建

在仓库的 `Actions` 页面打开 `Release build`，点击 `Run workflow` 并选择分支。手动运行会执行完整质量检查和双架构构建，产物只保存在本次运行的 Actions Artifact 中，不会创建公开 Release。

### 正式发布

1. 将 `package.json`、`src-tauri/tauri.conf.json` 和 `src-tauri/Cargo.toml` 的版本号更新为同一个版本。
2. 提交并推送版本变更。
3. 创建并推送版本标签：

   ```bash
   git tag v0.2.5
   git push origin v0.2.5
   ```

4. 工作流完成质量检查和两个架构的构建后，会创建对应的 Draft Release，并上传：

   - `Lilt_<version>_windows_amd64_setup.exe`
   - `Lilt_<version>_windows_arm64_setup.exe`

5. 在 GitHub 的 Draft Release 页面检查安装包，编辑或补充 Release Note，确认无误后点击 `Publish release`。

首版安装包未进行 Windows 代码签名，首次安装时可能出现 SmartScreen 提示。工作流不会读取 Provider 配置，也不会将运行时密钥写入构建日志或 Release 资产。

如果质量检查或任一架构构建失败，工作流不会创建 Draft Release。修复后可以重跑同一个标签，已有 Draft 会复用并覆盖同名资产；标签已经对应正式 Release 时，工作流会停止修改，需使用新的版本号和标签。

## TO-DO

- [ ] 个人词典导出

- [ ] 术语库导入

- [ ] PDF全文翻译集成

- [ ] 性能优化

## 鸣谢

- [ahpxex/open-dictionary](https://github.com/ahpxex/open-dictionary/releases)

- [ahpxex/Aictionary](https://github.com/ahpxex/Aictionary)

- [ZMGID/kivio](https://github.com/ZMGID/kivio)

- [PDFMathTranslate-next/PDFMathTranslate-next](https://github.com/PDFMathTranslate-next/PDFMathTranslate-next)

## 许可证

Lilt 软件本体采用 [GNU Affero General Public License v3.0](https://www.gnu.org/licenses/agpl-3.0.html)（AGPLv3）授权。

随软件分发的词典数据来自 [ahpxex/open-dictionary](https://github.com/ahpxex/open-dictionary)，采用与 [Creative Commons Attribution-ShareAlike 4.0 International](https://creativecommons.org/licenses/by-sa/4.0/)（CC BY-SA 4.0）兼容的许可。词典数据的署名和相同方式共享要求以数据来源的具体许可声明为准。
