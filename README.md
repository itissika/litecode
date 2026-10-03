<h1 align="center">
  <img src="./assets/wordmark.png" alt="LiteCode" width="240" />
</h1>

<p align="center">
  <b>A lightweight desktop workbench for coding agents.</b>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-yellow.svg" alt="License: MIT"></a>
  <img src="https://img.shields.io/badge/Rust-2024-orange.svg" alt="Rust 2024">
  <img src="https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-lightgrey.svg" alt="Platform">
  <img src="https://img.shields.io/badge/idle-~50MB-informational.svg" alt="~50MB idle">
  <a href="https://github.com/itissika/litecode/actions"><img src="https://github.com/itissika/litecode/actions/workflows/windows-sidecar.yml/badge.svg" alt="CI"></a>
</p>

<p align="center">
  <a href="./README.md">English</a> · <a href="./README.zh-CN.md">简体中文</a>
</p>

***

![LiteCode full workbench](./assets/screenshots/Full.webp)

## 🚀 Try it

1. [Download](https://github.com/itissika/litecode/releases/latest)
2. Paste one key

**You're all set!**

macOS isn't supported yet — desktop is Windows-only (Linux ships a headless bundle).

## Why LiteCode

<table>
  <tr>
    <td align="center" width="33%">
      <img src="./assets/why/switch.webp" width="300" alt="Model switcher"><br><br>
      <b>No lock-in</b> — any provider's key works; switch mid-conversation.
    </td>
    <td align="center" width="33%">
      <img src="./assets/why/layout.webp" width="300" alt="Free layout"><br><br>
      <b>Any layout</b> — primary, subagents, editor, knowledge: dock them however you work.
    </td>
    <td align="center" width="33%">
      <img src="./assets/why/knowledge.webp" width="300" alt="Live knowledge"><br><br>
      <b>Live knowledge</b> — status at a glance; click a reference and jump straight in.
    </td>
  </tr>
</table>

### Other features

**On-demand footprint** — \~50MB idle core; semantic search / LSP / remote load only when you ask.

**🔌 Provider**

* **One key per provider** — paste a key and go.
* *Tip: Claude isn't included — not permitted; I can add it if someone needs it.*

**🤖 Agent**

* **Define your own agents** — prompt, toolset, step budget: build the one you need; nobody knows you better than you do. The built-in agents are good too.
* **Custom tools & MCP, hot-reloaded** — build the tool you want and use it right away; the generic ones don't deserve you.
* **LSP feedback, automatic** — a bad edit is reported to the agent in the same turn; neither of you has to hunt for it.
* **Thinking & context, one click** — 3 thinking levels, 2 context tiers; auto-compaction when the window overflows, so work never stops.
* **Whole-workspace session search** — a memory add-on for your agent; anything said can be pulled back up.

**🧠 Knowledge**

* **Nodes with stable references** — pull one thread and the whole picture moves; see everything from one node and steer like a strategist.
* **Knowledge that stays alive** — it follows the facts and never misses an edit you or the agent make to files or code; your notes don't rot.

**🖥️ IDE**

* **File tree + editor** — light code editing; with LSP on, the full editing experience.
* **Preview anything** — text, code, images, PDF, SQLite: click and look.

## 🧭 Design

LiteCode is built for one developer, one workspace. **We only provide tools** — the agent is not omnipotent, and it is not you; only you know what you want to do. So it deliberately skips multi-workspace windows, scheduled wake-up tasks, skills and convenience pile-ups.

The design rationale lives in the repo's own knowledge base — `knowledge/`, the same feature the product ships. Start at [Why](./knowledge/why.md) → [Product brief](./knowledge/product-brief.md) → [Harness philosophy](./knowledge/harness/agent-harness-philosophy.md), then the [featured tools](./knowledge/framework/featured-tools.md) it lands on. (Notes are written in Chinese.)

## 🧑‍💻 Development

```powershell
# Windows desktop (Electron host + sidecar)
./scripts/dev_win.ps1
```

```bash
# Linux / browser (Vite HMR)
./scripts/serve.sh
```

> Prerequisites: Rust (MSVC, edition 2024) + Node.js 22+.

Local nightly (Windows installers + slim Linux tar, `LITECODE_CHANNEL=nightly`):

```powershell
./scripts/package_local.ps1
```

Artifacts: `desktop/out/` and `dist/linux/`. Official signed builds come from GitHub Releases.

## 📚 Advanced

<details>
<summary>Project structure</summary>

```
src/
  agent/            Agent loop & dispatch (frozen control flow)
  tool/             Tool pipeline: one execution / authorization / output contract
  tools/            Built-in toolset (read / grep / edit / bash / subagent / knowledge …)
  context_pipeline/ Context views, compaction & truncation
  session/          Session log (seq-authoritative), snapshots & revert
  knowledge/        Knowledge base: corpus, refs, validation
  engines/          Semantic search / ANN / LSP lifecycle
  llm/              LLM adapters (OpenAI Responses canonical)
  provider_catalog/ Provider & model catalog, compiled into the build
  runtime/          Runtime handle & provider resolution
  permission/       Permissions & sensitive-path guards
  reminder/         System reminders, written into the session as facts
  terminal/         PTY base
  workspace/        Workspace substrate (files / git / shared infrastructure)
  mcp/              MCP servers
  serve/            HTTP/WS backend
  client_protocol/  JSON-RPC 2.0 client protocol
web/                React UI (Monaco + dockview)
desktop/            Electron host (sidecar + SSH remote)
examples/tools/     Custom tool examples
models/             Embedded embedding weights (shipped with the repo)
knowledge/          LiteCode's own design knowledge (dogfooded)
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

Configuration: after `serve` starts, manage providers, models and agents via the web settings UI.

</details>

## Contributing

Vibe-coded, bugs happen — I did my best. Issues and PRs are welcome.

* Project contract & commit rules: [AGENTS.md](AGENTS.md)
* Contribution guide: [CONTRIBUTING.md](CONTRIBUTING.md)
* Changelog: [CHANGELOG.md](CHANGELOG.md)
* Desktop details: [desktop/README.md](desktop/README.md)

## License

[MIT](LICENSE) © LiteCode contributors
