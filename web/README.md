# litecode Web UI

First-party reference client for the Rust kernel. Business logic lives in Rust
(`litecode serve`); this directory visualizes agent, permission, session, and
workspace capabilities. It does not implement agent policy or tool semantics in
the browser.

## Role

| Layer | Responsibility |
|-------|----------------|
| **RS** | Agent loop, session, permission, tool, hook |
| **serve** | WebSocket + REST + static `web/dist` |
| **web/** | Three-pane workbench: agent sidebar, file tree, Monaco editor |

When the kernel grows via **tools / hooks**, the UI adds matching presentation
(tool cards, settings, telemetry) rather than a parallel implementation.

## Prerequisites

```bash
# repo root
cargo run -- serve
```

Default: `http://127.0.0.1:7483` (also serves `web/dist` after `npm run build`).

## Development

```bash
cd web
npm install
npm run dev
```

Open [http://localhost:5173](http://localhost:5173). Vite proxies `/ws`, `/health`,
and `/api` to `127.0.0.1:7483`.

**Windows:** one-shot API + Vite (prints a handshake URL):

```powershell
# repo root
./scripts/serve_win.ps1
# open the printed LITECODE_BROWSER_DEV link (includes ?token=)
```

End-state Electron shell: `./scripts/dev_win.ps1` (no Vite HMR).

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

0. **Settings** — connection, models, tool catalog, default agent, agents (per-tool bindings), advanced (auth + log); save is disabled while a turn is running
1. **Explorer (left)** — lazy file tree; click to open
2. **Editor (center)** — Monaco; **Ctrl+S** saves (`PUT /api/workspace/file`)
3. **New / Delete** — explorer **+** and context-menu delete
4. **Agent (right)** — streaming chat, tool cards, permission prompts, session list (New / Delete)
5. **Sync** — `workspace_changed` refreshes open tabs; `read` / `write` / `edit` tool paths open on click

## Layout (`web/src/`)

```
api/          types.ts, agentWs.ts, adapter.ts, workspace.ts, settings.ts
stores/       agentStore, editorStore, treeStore, settingsStore
components/   AppShell, AgentSidebar, SettingsPanel, FileTree, EditorPane, …
```

`adapter.ts` maps the RS wire protocol to UI message parts. **Do not use the AI SDK as the wire format.**

## Build

```bash
npm run build    # emit web/dist for serve static hosting
npm run preview
npm test         # Vitest: adapter fixtures + merge regressions
```

## Stack

- Vite 6 + React 19 + TypeScript
- Tailwind CSS v4 + Zustand
- Monaco Editor、`react-markdown` + `remark-gfm`
