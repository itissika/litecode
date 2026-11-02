# 构建与打包

| 态 | 入口 | 产物 | 备注 |
|---|---|---|---|
| 开发（桌面） | `scripts/dev_win.ps1` | `dist/product` + Electron 拉起 sidecar | 改过 web 加 `-RebuildWeb`；纯本地 `-SkipLinuxBundle` |
| 开发（浏览器） | `scripts/serve_win.ps1` / `scripts/serve.sh` | API + Vite | 终端打印带 token 的握手 URL |
| nightly | `scripts/package_local.ps1` | slim NSIS + WSL Linux tar + `dist/components` | 不发 Release |
| official | `scripts/package_win.ps1` → tag → Publish | 签名安装包 + 构件 | 走 `windows-signed-release` |

## 注意

- WSL checkout 必须与 Windows 同一 commit / 版本。
- `-Full`（内嵌模型 + Linux tar）只用于本地离线，release CI 永不用。
- 离线验首启下载：`$env:LITECODE_COMPONENTS_BASE = "<repo>\dist\components"`。
- 构建前置：`web/`、`desktop/` 先 `npm install`。
