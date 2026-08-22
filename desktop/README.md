# Litecode Desktop (Electron)

Host shell for the `litecode` sidecar. Users never type a serve auth token for
local workspaces; remote sessions generate a token automatically and show it
read-only.

## Dev (Windows — preferred, end-state shell)

One-shot PowerShell loop: assemble `dist/product` (debug cargo + `web/dist`) and launch Electron so it spawns the sidecar with auth injection — same shape as a packaged install, not the browser Vite path (`scripts/serve.sh` / `scripts/serve_win.ps1`).

```powershell
# From repo root
./scripts/dev_win.ps1
./scripts/dev_win.ps1 -RebuildWeb     # after UI changes
./scripts/dev_win.ps1 -SkipAssemble   # reuse existing dist/product
```

For UI HMR in a normal browser (transition), use `./scripts/serve_win.ps1` instead — it prints a `LITECODE_BROWSER_DEV` handshake URL with `?token=`.

## Dev (Linux / manual)

1. Build or assemble a sidecar tree:

```bash
# Linux
LITECODE_PROFILE=debug LITECODE_BUILD_WEB=0 LITECODE_BUNDLE_MODEL=0 ./scripts/assemble_product.sh
```

```powershell
# Windows (manual steps; prefer scripts/dev_win.ps1)
./scripts/assemble_product.ps1 -Profile debug -SkipWeb -SkipModel
```

2. Install and run the shell:

```bash
cd desktop
npm install
npm run dev
```

Optional automation attach (no Home UI). Requires the same
`LITECODE_TOKEN` the remote `serve --require-auth` uses:

```bash
# Formal
LITECODE_REMOTE_URL=http://127.0.0.1:7483 LITECODE_TOKEN=<token> npm run dev

# Legacy alias (same semantics)
LITECODE_DEV_URL=http://127.0.0.1:7483 LITECODE_TOKEN=<token> npm run dev
```

In the packaged app: use the Home hub for workspace switching.
Workbench **Options → Home** returns to that hub.

Browser / `serve_win.ps1` handshake URLs are **DEV only**, not the product shell.

## Startup hub

Normal desktop launches open the startup hub instead of requiring a folder picker.

Home exposes exactly two open actions:

- **Open local** — pick a folder; Electron spawns the local sidecar with an injected token.
- **Open remote** — `user@host` + password (advanced: private key / agent) → automatic
  Linux server deploy with progress → choose a remote workspace folder → show the
  auto-generated session token (read-only / copy) → enter the workspace.

Histories:

- **Local history** — recent local folders; one click reopens.
- **Remote history** — only remotes that successfully attached at least once; one click
  reconnects SSH, ensures the server, and opens the last workspace.

`--workspace <path>` and `LITECODE_WORKSPACE` remain automation/deep-link escapes that
open a local workspace directly.

The first connected Litecode server (local or remote) must have a configured
Provider, at least one model, and a default-agent model before the workbench
is shown. These settings are server-global: remote provider credentials are
entered and stored on the remote host; they are never copied from the local
machine.

### Remote path (managed SSH)

Open remote is SSH orchestration owned by `DesktopHost`:

1. Authenticate to the host (password primary; key/agent optional).
2. Upload/verify/extract the bundled Linux server tar under the remote home (progress events).
3. Start a loopback-only serve for the chosen workspace and forward a local port.
4. Persist host credentials in Electron `safeStorage` and the workspace path in remote history
   only after a successful attach.

Closing the client stops that temporary remote serve and tunnel. A separate
“deployed URL + hand-typed token” product path is not shown in Home; use
`LITECODE_REMOTE_URL` + `LITECODE_TOKEN` for automation.

Windows packaging requires `dist/linux/litecode-server-linux-x64.tar.gz` and its
`.sha256` (built on Linux via `scripts/package_linux.sh`); these are embedded in the
Windows app for managed SSH upload.

Local nightly slim SKU (no models, no embedded tar):

```powershell
./scripts/package_local.ps1
./scripts/package_local.ps1 -WslRoot /home/<you>/litecode
```

Artifacts land under `dist/` (Linux tar) and `desktop/out/` (Portable / NSIS).
Open Remote then reads the tar from `LITECODE_BUNDLE_ROOT/linux/` or
`%LOCALAPPDATA%\litecode\bundles\linux\`; embed weights from `LITECODE_MODEL_DIR`
or the same bundle root’s `models/` tree.

## Production packaging (Windows x64)

```powershell
# From repo root on Windows (or windows-latest CI)
./scripts/package_win.ps1
```

Outputs under `desktop/out/`:

- `Litecode-Portable-*-x64.exe` (unsigned portable)
- `Litecode-Setup-*-x64.exe` (unsigned NSIS)

### Code signing (W4)

Set secrets / env before `electron-builder`:

| Variable | Purpose |
|----------|---------|
| `CSC_LINK` | Path or base64 of `.pfx` |
| `CSC_KEY_PASSWORD` | PFX password |
| `WIN_CSC_LINK` / `WIN_CSC_KEY_PASSWORD` | Windows-specific aliases |

When these are unset, builds stay unsigned (`signExecutable: false`) but still embed the app icon into the `.exe`. The release workflow only attaches installers to a GitHub Release when signing env is present.

## Contracts

- One Electron window owns one workspace process (VS Code model)
- Spawns `litecode --workspace <path> serve --bind 127.0.0.1:0 --require-auth --parent-pid <pid>` with `cwd=<workspace>`
- Injects `LITECODE_TOKEN`; preload exposes `window.litecode.getAuthToken`
- Sidecar / `server/hello.project` paths are Litecode Absolute Path (LAP): canonical, no Windows `\\?\` prefix
- Desktop `normalizeWorkspace` (registry + sidecar spawn) matches LAP: realpath + strip verbatim + uppercase drive; `hello.project` is the authoritative root after connect
- Frameless window: Litecode `TitleBar` owns minimize / maximize / close via preload IPC
- Embed model lives under `sidecar/models/` (tens of MB on disk; ORT Session only loads when `code_search` warms)
- `pickFolder` / `focusWorkspace` / `notifyWorkspace` / `openWorkspace` (sidecar relaunch + window reload)
- Home remote: `startRemoteSession` / `completeRemoteSession` / `enterRemoteWorkbench` / `reconnectRemote`
- Multiple app instances allowed; same workspace focuses existing window when possible
- Changing folder relaunches the sidecar (no in-process hot switch)
