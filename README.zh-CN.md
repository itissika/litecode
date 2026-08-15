<h1 align="center">
  <img src="./assets/wordmark.png" alt="LiteCode" width="240" />
</h1>

<p align="center">
  <b>追求运行时极致轻量的 Coding Agent 框架</b>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Rust-2024-orange.svg" alt="Rust 2024">
  <img src="https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-lightgrey.svg" alt="Platform">
  <a href="https://github.com/itissika/litecode/actions"><img src="https://github.com/itissika/litecode/actions/workflows/windows-sidecar.yml/badge.svg" alt="CI"></a>
</p>

---

一坨 Vibe Coding 出来的、专门针对 Coding 场景的 Agent 框架。专注于：**Tool 和 Context 这些经常对人类不可见但是 agent 主战场的实现**，当然，还有我最喜欢的前端主题和动画，花了我超多时间。

![LiteCode 完整工作台](./assets/screenshots/Full.png)

## ✨ 功能亮点

<p align="center">
  <img width="220" height="220" src="./assets/features/c-lightweight.webp" alt="极致轻量">
  <img width="220" height="220" src="./assets/features/c-search.webp" alt="搜索面板">
  <img width="220" height="220" src="./assets/features/c-sessions.webp" alt="Session 并行">
  <img width="220" height="220" src="./assets/features/c-switch.webp" alt="运行中切换">
</p>

## 🚀 快速开始

### Windows

前往 [Releases](https://github.com/itissika/litecode/releases/latest) 下载安装包：

- `Litecode-Setup-x64.exe` — 安装版（推荐）
- `Litecode-Portable-x64.exe` — 免安装便携版

首次启动后在 Web 设置界面配置 Provider → Model → 默认 Agent，凭据只存本机。

### 其他平台

Linux 提供无头服务端包（`litecode-server-linux-x64.tar.gz`，供 SSH 远程部署），无桌面安装包；macOS 暂无任何安装包。两者均需从源码构建（见下方「开发」）。

## 🧑‍💻 开发

```powershell
# Windows 桌面端（Electron 宿主 + sidecar）
./scripts/dev_win.ps1
```

```bash
# Linux / 浏览器端（Vite 热更）
./scripts/serve.sh
```

```bash
# CLI（开发便利）
cargo run -- "帮我修复这个 bug"
```

> 前置要求：Rust（MSVC，edition 2024）+ Node.js 22+。

## 📚 进阶

<details>
<summary>项目结构</summary>

```
src/
  agent/            Agent 定义与调度
  client_protocol/  JSON-RPC 2.0 客户端协议
  context_pipeline/ 上下文压缩与截断
  engines/          语义搜索 / ANN / LSP
  llm/              LLM 适配器（OpenAI Responses）
  permission/       权限与敏感路径防护
  runtime/          运行时与 provider 解析
  serve/            HTTP/WS 后端
  session/          Session 存储与快照
  tools/            工具集（grep / write / webfetch / subagent …）
  workspace/        工作区抽象（LAP）
web/                React UI（Monaco + dockview）
desktop/            Electron 宿主（sidecar + SSH 远程）
examples/tools/     自定义工具示例
models/             嵌入模型权重（随仓分发）
scripts/            开发与打包脚本
```

</details>

<details>
<summary>完整构建 & 配置</summary>

```bash
# Rust 核心
cargo build --release

# Web UI
cd web && npm install && npm run build

# Desktop 壳
cd desktop && npm install && npm run build
```

配置入口：`serve` 启动后通过 Web 设置界面管理 Provider（openai / deepseek / mimo）、Model、Agent。

</details>

## 参与贡献

- 项目契约与提交铁律：[Agent.md](Agent.md)
- 贡献流程：[CONTRIBUTING.md](CONTRIBUTING.md)
- 版本变更：[CHANGELOG.md](CHANGELOG.md)
- 桌面端细节：[desktop/README.md](desktop/README.md)

## License

[MIT](LICENSE) © LiteCode contributors
