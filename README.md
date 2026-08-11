# Lilt

Lilt 是面向中文用户的桌面阅读辅助工具。当前版本聚焦段落翻译，前端保持基础功能形态，先验证翻译链路、流式状态、历史记录和段落缓存。

## 开发环境

- Node.js 20 或更高版本
- Rust stable MSVC 工具链
- Windows WebView2 Runtime

如果当前终端没有自动识别 Rust，可以在 PowerShell 中临时补充 Cargo 路径：

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

## 常用命令

```bash
npm install
npm run dev
npm run tauri -- dev
npm run typecheck
npm run lint
npm test -- --run
npm run build
npm run tauri -- build
```

Rust 检查命令：

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

开发版后端诊断：

```powershell
npm run tauri -- dev -v
```

Provider 请求阶段、HTTP 状态、超时、重试和翻译终态会输出到启动该命令的终端。日志不包含 API Key、Authorization、原文、Prompt 或响应正文。发布版本地日志暂未实现。

## 目录约定

- `src/`：React 页面、前端事件契约和样式
- `src-tauri/src/`：Provider、SQLite、凭据存储、翻译编排和 Tauri 命令
- `src-tauri/tauri.conf.json`：桌面窗口与打包配置
- `.trellis/tasks/08-11-lilt-product-technical-plan/`：本次产品方案、设计和执行记录

API Key 只由 Rust 侧写入 Windows 凭据管理器，SQLite、翻译事件、历史、缓存和日志不保存 API Key。段落缓存默认开启，历史记录始终开启，二者的设置含义保持独立。
