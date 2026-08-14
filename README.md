# Lilt

Lilt 是面向中文用户的桌面阅读辅助工具。当前版本围绕段落翻译、离线词典和 Windows 划词翻译展开，重点验证翻译链路、流式状态、缓存、历史记录以及桌面交互。

当前配置版本为 `0.2.0`。项目仍处于首版收尾阶段，Windows 是主要开发和验证平台。

## 已实现功能

### 段落翻译

- 支持英语、简体中文、繁体中文、日语和韩语之间的段落翻译。
- 支持 OpenAI 兼容 Provider，能够获取模型列表并在设置中选择模型。
- 支持流式输出、取消请求、复制译文，以及显示翻译耗时和缓存命中状态。
- 翻译历史始终开启，可设置保留条数；历史记录通过译文操作区旁的按钮打开。
- 段落翻译缓存默认开启，设置页显示占用空间，并允许调整最大存储空间。
- 翻译提示词支持内置 Prompt 和自定义 Prompt，可创建、编辑、复制、设为默认或删除。
- 支持 `none`、`low`、`medium`、`high` 四档思考强度，其中 `none` 也会明确传给 Provider。

思考强度不参与翻译缓存的匹配条件。只要文本和相关翻译参数匹配，缓存就可以命中。

### 词典

- 使用本地 open-dictionary 数据库，支持精确查询和大小写不敏感查询。
- 支持词形变化查询；当一个词形对应多个规范词头时，界面会让用户选择目标词头。
- 本地展示释义、词性、发音、例句、词义关系和少见义项等信息。
- 记录最近查询及查询次数，词典页面提供个人词典入口。
- 支持收藏和取消收藏，个人词典单独展示，并按批次加载条目。
- 词典数据不随安装包内置。首次使用时从 `ahpxex/open-dictionary` 的 GitHub Release 下载，安装到应用数据目录；更新时显示下载进度，并在安装前校验数据。

### 单词 AI 见解

- 如果单词出现在段落翻译缓存中，且所在句不超过 15 个词，可以从缓存例句查询库中找到原句。
- 例句查询库在段落缓存完成后异步构建，每个单词保留最新的五条例句。
- AI 见解流式生成例句翻译和词性；生成完成前先显示原例句。
- 单词 AI 见解缓存和从段落缓存中查找例句均可在设置中独立关闭。

### 划词翻译与桌面交互

- Windows 下使用 UI Automation 捕获选区，支持快捷键和自动触发两种模式。
- 默认快捷键为 `Ctrl+Shift+L`，自动触发要求选区稳定 500 毫秒；快捷键和触发方式均可调整。
- 先显示纯色圆形触发按钮，用户点击后才提取文字并发起请求。
- 单词查询和段落翻译会自动路由到对应功能，结果显示在可拖动、可调整大小的小窗中。
- 小窗支持流式输出、取消、复制和打开主窗口；当前未启用剪贴板回退方案。
- 主窗口使用无边框布局，设置、翻译历史等内容以浮动窗口呈现。
- 支持系统托盘。关闭主窗口时可以选择退出、缩小到托盘或每次询问，并可记住选择。

### 其他功能

- 术语表支持添加原文、译文和备注，条目列表按批次加载。
- 主窗口和托盘使用项目根目录的 `lilt_logo.svg` 作为图标源。
- Windows 托盘图标在启动时根据系统 DPI 从 ICO 资源中选择合适的图层；调整系统 DPI 后，重启应用即可重新选择资源。

## 技术架构

- 前端：React、TypeScript、Vite。
- 桌面运行时：Tauri 2。
- 后端：Rust，负责 Provider 请求、流式翻译、缓存、历史、词典、设置、系统托盘和桌面命令。
- 数据库：SQLite，使用 bundled SQLite，保存本地历史、缓存、术语表、个人词典和词典索引等数据。
- 网络请求：Rust 侧使用 reqwest，Provider 协议当前采用 OpenAI 兼容接口。
- Windows 选区：Rust 侧使用 UI Automation。

主要目录：

- `src/`：React 页面、交互逻辑、事件订阅和样式。
- `src-tauri/src/`：Provider、SQLite、Tauri 命令、缓存、词典、托盘和选区处理。
- `src-tauri/tauri.conf.json`：桌面窗口、开发服务和打包配置。
- `src-tauri/icons/`：桌面和托盘图标资源。
- `lilt_logo.svg`：项目统一的 Logo 源文件。

## 数据与安全

- API Key 只由 Rust 侧写入 Windows 凭据管理器。
- API Key 不写入 SQLite、翻译事件、历史记录、缓存或开发日志。
- 开发日志只输出到启动开发版的终端，不保存本地日志文件。
- Provider 请求日志包含请求 ID、请求阶段、模型、状态、重试和终态等诊断信息，不包含 API Key、Authorization、原文、Prompt 或响应正文。
- 翻译历史始终开启；段落翻译缓存、单词 AI 见解缓存和例句查询开关分别管理，互不改变各自的语义。

## 开发环境

- Node.js 20 或更高版本
- Rust stable MSVC 工具链
- Windows WebView2 Runtime
- Windows 10 或更高版本，建议使用 Windows 11 进行开发和验证

如果当前 PowerShell 会话没有自动识别 Rust，可以临时补充 Cargo 路径：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

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

首次运行后，在设置中配置 OpenAI 兼容 Provider、Base URL、API Key 和模型。词典数据需要在设置页单独下载。

## 当前未纳入范围

- PDF 集成暂缓，待首版其他功能稳定后再规划。
- TTS 不在当前产品范围内。
- IA2 支持和剪贴板选区回退方案尚未实现，当前 Windows 划词依赖 UI Automation。
- 字典输入联想和全词库预测下拉框暂未启用，当前以精确查询为主。
- 托盘图标只在应用启动时根据 DPI 选择资源，运行中跨显示器切换不会实时重选。
- 发布版本的本地日志文件暂未实现。
