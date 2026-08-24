<h1 align="center">
  <img src="./assets/wordmark.png" alt="LiteCode" width="240" />
</h1>

<p align="center">
  <b>A coding agent framework obsessively optimized for runtime lightness</b>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Rust-2024-orange.svg" alt="Rust 2024">
  <img src="https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-lightgrey.svg" alt="Platform">
  <a href="https://github.com/itissika/litecode/actions"><img src="https://github.com/itissika/litecode/actions/workflows/windows-sidecar.yml/badge.svg" alt="CI"></a>
</p>

---

A vibe-coded agent framework built specifically for coding. It focuses on **Tools and Context — the parts usually invisible to humans, yet where agents do the real work** — plus, of course, my favorite frontend theme and animations, which took an absurd amount of time.

![LiteCode full workbench](./assets/screenshots/Full.webp)

## ✨ Feature Highlights

**On-demand footprint** — ~50MB baseline core, ultra-light by default; stack semantic search / LSP / remote on demand — as heavy as you need.

### 🤖 Agent
- **Parallel execution** — Multiple sessions run in parallel with concurrent subagents (non-nested), non-blocking.
- **Free orchestration** — Add primary / sub agents as needed, each with its own toolset and system prompt.
- **Extensible tools** — Custom tools and MCP servers on top of the built-in toolset; register and use.
- **Hot-plug tools** — Settings take effect on the next turn without restarting serve.
- **Full LSP experience** — With LSP on, write / edit get automatic diagnostics feedback — the agent edits like a human.
- **Safety policies** — One-click preset switch for tool authorization; sensitive-path protection; session snapshots with mid-run revert.
- **Context compression** — auto or one-click manual; keep-recent automatically keeps key content; built-in `session_search` tool recalls history losslessly.

### 🖥️ IDE
- **Lightweight editing** — Embedded Monaco editor, ready out of the box; IDE capabilities are Agent capabilities, prioritized for the agent side.
- **Semantic search** — Code search by meaning, not text (200+ languages, ANN + tree-sitter).
- **Two form factors + remote** — Electron desktop and browser; SSH to a headless Linux server, tunneled back.

### 🔌 Provider
- **Multi-adapter** — OpenAI Responses for openai / deepseek / mimo; OpenCode Chat Completions with Zen by default and an optional Go endpoint.
- **Mid-turn switching** — Swap model / agent anytime without losing context; effective next turn.
- **Cost visibility** — prompt / completion / cache hit / miss tracked per step in real time for precise cost control.

## 🚀 Quick Start

### Windows

Download from [Releases](https://github.com/itissika/litecode/releases/latest):

- `Litecode-Setup-x64.exe` — installer (recommended)
- `Litecode-Portable-x64.exe` — portable

After first launch, configure Provider → Model → default Agent in the web settings. Credentials stay local.

### Other platforms

Linux ships a headless server bundle (`litecode-server-linux-x64.tar.gz`, for SSH remote deployment) with no desktop installer; macOS has no packages yet. Both must be built from source (see Development below).

## 🧑‍💻 Development

```powershell
# Windows desktop (Electron host + sidecar)
./scripts/dev_win.ps1
```

```bash
# Linux / browser (Vite HMR)
./scripts/serve.sh
```

```bash
# CLI (dev convenience)
cargo run -- "fix this bug for me"
```

> Prerequisites: Rust (MSVC, edition 2024) + Node.js 22+.

## 📚 Advanced

<details>
<summary>Project structure</summary>

```
src/
  agent/            Agent definition & dispatch
  client_protocol/  JSON-RPC 2.0 client protocol
  context_pipeline/ Context compression & truncation
  engines/          Semantic search / ANN / LSP
  llm/              LLM adapters (OpenAI Responses)
  permission/       Permissions & sensitive-path guards
  runtime/          Runtime & provider resolution
  serve/            HTTP/WS backend
  session/          Session storage & snapshots
  tools/            Toolset (grep / write / webfetch / subagent …)
  workspace/        Workspace abstraction (LAP)
web/                React UI (Monaco + dockview)
desktop/            Electron host (sidecar + SSH remote)
examples/tools/     Custom tool examples
models/             Embedded model weights (shipped with the repo)
scripts/            Dev & packaging scripts
```

</details>

<details>
<summary>Full build & configuration</summary>

```bash
# Rust core
cargo build --release

# Web UI
cd web && npm install && npm run build

# Desktop shell
cd desktop && npm install && npm run build
```

Configuration: after `serve` starts, manage Provider (openai / deepseek / mimo / opencode), Model, and Agent via the web settings UI.

</details>

## Contributing

- Project contract & commit rules: [Agent.md](Agent.md)
- Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
- Changelog: [CHANGELOG.md](CHANGELOG.md)
- Desktop details: [desktop/README.md](desktop/README.md)

## License

[MIT](LICENSE) © LiteCode contributors
