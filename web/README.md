# litecode Web UI

**RS 内核的第一方参考客户端。** 业务真源在 Rust（`litecode serve`）；本目录负责把 agent、权限、会话、工作区能力**可视化**，不在浏览器中实现 agent 策略或工具语义。

## 定位

| 层 | 职责 |
|----|------|
| **RS** | Agent Loop、Session、Permission、Tool、Hook |
| **serve** | WebSocket + REST + 静态 `web/dist` |
| **web/** | 三栏 MCE：Agent 侧栏、文件树、Monaco 编辑器 |

后续 RS 通过 **Tool / Hook** 增厚时，Web 侧增加对应展示（工具卡、配置、观测），而非平行实现逻辑。

## Prerequisites

```bash
# 仓库根目录
cargo run -- serve
```

默认：`http://127.0.0.1:7483`（若已 `npm run build`，同时托管 `web/dist`）

## Development

```bash
cd web
npm install
npm run dev
```

打开 [http://localhost:5173](http://localhost:5173)。Vite 将 `/ws`、`/health`、`/api` 代理到 `127.0.0.1:7483`。

**Windows：** 一键 API + Vite（终端打印握手 URL）：

```powershell
# 仓库根目录
./scripts/serve_win.ps1
# 打开打印的 LITECODE_BROWSER_DEV 链接（含 ?token=）
```

终态 Electron 壳用 `./scripts/dev_win.ps1`（无 Vite HMR）。

### Environment variables

| Variable | Description |
|----------|-------------|
| `VITE_WS_URL` | Override WebSocket URL when not using the Vite proxy (e.g. `ws://127.0.0.1:7483/ws`) |
| `VITE_AUTH_TOKEN` | **Dev only.** Must match server `LITECODE_TOKEN`; sent as query `?token=` and as the `auth` wire frame on connect. Prefer starting via `scripts/serve_win.ps1` / `serve.sh`, which inject matching tokens. You can also open a handshake URL with `?token=` (read by `getAuthToken`). Production builds served by `litecode serve` should rely on host-injected auth — do not embed secrets in the client bundle. |

**Server auth:** set `LITECODE_TOKEN` in the environment when starting `cargo run -- serve`. There is no `--auth-token` CLI flag.

### Authentication

When `LITECODE_TOKEN` is set on the server:

1. Set **`VITE_AUTH_TOKEN`** to the same value in `web/.env` or your shell before `npm run dev`.
2. The client sends the token in the WS URL query **and** an `{ "auth": { "token": "..." } }` frame on `onopen`.

If the token is wrong or missing, the L1 handshake watchdog (2s without `server_hello`) stops reconnect and shows an explicit error. Browsers cannot read HTTP 401 from the WS upgrade; use the curl probe below for backend verification.

#### Silent discard (connected but no agent response)

If `LITECODE_TOKEN` is set but you only put `?token=` in `VITE_WS_URL` **without** `VITE_AUTH_TOKEN`:

- The UI may show **connected** and receive `server_hello` / `session_snapshot`.
- Non-`auth` requests (`start`, `list_sessions`, etc.) are **silently ignored** until post-connect auth completes.

Always configure **`VITE_AUTH_TOKEN`** so the client sends the `auth` frame.

#### L4 curl probe (optional)

With `LITECODE_TOKEN=secret cargo run -- serve`:

```bash
# Expect HTTP/1.1 401
curl -i -N \
  -H "Connection: Upgrade" \
  -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  "http://127.0.0.1:7483/ws?token=wrong"

# Expect HTTP/1.1 101 Switching Protocols
curl -i -N \
  -H "Connection: Upgrade" \
  -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" \
  "http://127.0.0.1:7483/ws?token=secret"
```

## Usage

0. **Settings（⚙）** — 六区块设置面板：连接、模型、Tool 目录、默认 Agent、Agents（per-tool 绑定）、高级（auth + log）；turn 进行中禁用保存
1. **Explorer（左）** — 懒加载文件树；点击打开文件
2. **Editor（中）** — Monaco 高亮；**Ctrl+S** 保存（`PUT /api/workspace/file`）
3. **New / Delete** — 资源管理器 **+** 与右键删除
4. **Agent（右）** — 流式对话、工具卡、权限弹窗、会话列表（New / Delete）
5. **联动** — `workspace_changed` 刷新已打开 tab；`read`/`write`/`edit` 工具路径可点击打开

## Layout (`web/src/`)

```
api/          types.ts, agentWs.ts, adapter.ts, workspace.ts, settings.ts
stores/       agentStore, editorStore, treeStore, settingsStore
components/   AppShell, AgentSidebar, SettingsPanel, FileTree, EditorPane, …
```

`adapter.ts` 将 RS 线协议映射为 UI 消息部件；**不使用 AI SDK 作为 wire format**。

## Build

```bash
npm run build    # 输出 web/dist，供 serve 静态托管
npm run preview
npm test         # Vitest：adapter fixture + merge 回归
```

## Stack

- Vite 6 + React 19 + TypeScript
- Tailwind CSS v4 + Zustand
- Monaco Editor、`react-markdown` + `remark-gfm`
