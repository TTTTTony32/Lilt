<p align="center">
  <img src="source/banner.jpg" alt="Lilt" />
</p>

<h1 align="center">Lilt</h1>

<p align="center">Lilt 是面向中文用户的桌面阅读辅助工具</p>

## 主要功能

### 段落翻译

- 支持英语、简体中文、繁体中文、日语和韩语
- 基于LLM服务进行翻译，目前暂时仅支持 `OpenAI Completions API`
- 提示词支持内置 Prompt 和自定义 Prompt

### 词典

- 使用 [ahpxex/open-dictionary](https://github.com/ahpxex/open-dictionary) 词典库
- 支持收藏与个人词典 UTF-8 TXT 导出
- 查询词若曾在段落翻译中翻译过，所在句将自动作为例句出现

### 划词翻译

- 支持快捷键和自动触发两种模式
- 单词查询和段落翻译会自动路由到对应功能，结果显示在悬浮窗中

### 其他功能

- 术语表：支持手动添加原文-译文对和备注，支持 CSV 导入与导出
- PDF全文翻译（基于 [PDFMathTranslate-Next](https://github.com/PDFMathTranslate-next/PDFMathTranslate-next) ）（暂未做完）

## 技术架构

- 前端：React、TypeScript、Vite
- 桌面运行时：Tauri 2
- 后端：Rust
- 数据库：SQLite

## 开发环境

- Node.js 24 或更高版本
- Rust 1.85 或更高版本，使用 Rust 2024 edition 的 MSVC 工具链
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
cargo check --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

构建前端和桌面安装包：

```bash
npm run build
npm run tauri -- build
```

## TO-DO

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
