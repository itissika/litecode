# WP-185 desktop 同契约审计（父 WP-182）

- 包：WP-185（owner `frontend_desktop`，审批 `frontend_mid`），父包 WP-182
- 时间：2026-10-16 ~07:45 HKT（Asia/Hong_Kong）
- 范围：只读核对 `desktop/`（src + scripts + README）；**未**改 Rust / web 产品码；**未**跑 tsc / 单测 / npm / cargo；**未**占桌面；**未** commit
- 依据：WP-182 plan §5；前端中层指派「agent 面板 = 内嵌 web，同 `pending_permissions` / `waiting_for_user`」

## 结论

**无分叉。** desktop 壳没有第二套 `pending_permissions` / `waiting_for_user` 字段，也不解析、不改写 `SessionSnapshot`。Agent 面板 UI 与协议消费全在内嵌 web bundle（WP-184）；壳只负责加载 workbench URL。

**透传：** 壳侧**不**转发 session snapshot。Workbench 经 handshake URL 直连 sidecar `/api` `/ws`；权限挂起列表由 web 按契约 hydrate，桌面无中间层改写点。

## 证据（可复跑）

```bash
cd /workspace/litecode
rg -n 'pending_permissions|waiting_for_user' desktop/          # 0 命中
rg -n 'SessionSnapshot|PermissionRequest|permissionQueue' desktop/src   # 0（协议型）
rg -n 'setPermissionRequestHandler|setPermissionCheckHandler' desktop/src
```

| 检查项 | 结果 | 路径 |
|--------|------|------|
| 契约字段 `pending_permissions` / `waiting_for_user` | **无** | 全 `desktop/` rg = 0 |
| Agent 面板宿主 | 内嵌 web：`safeLoadURL` / `loadWorkbench` → handshake / Vite / `web/dist` | `desktop/src/main.ts`（`loadLocalWorkbench`、`safeLoadURL`）；`desktop/src/boot-sequence.ts`；`desktop/README.md` |
| 壳桥 `window.litecode` | 无 session / permission 协议键；仅远程进度、组件、BrowserView、layout、auth token 等壳 IPC | `desktop/src/bridge-contract.ts`、`preload.ts` |
| 名为 snapshot 的代码 | **非** `SessionSnapshot`：① BrowserView 状态快照；② dockview **布局** JSON 落盘（原文/verbatim） | `browser-host.ts` `snapshot()`；`layout-file.ts`（注释写明 dockview layout） |
| Electron permission handler | Chromium **访客页**权限（一律 deny），与 agent `PermissionRequest` 无关 | `desktop/src/browser-view.ts:23-26` |

## 与 WP-182 plan §5 对照

| plan 要求 | 本包 |
|-----------|------|
| 无独立 PermissionRequest 协议 | 满足：壳无该类型/字段 |
| agent 面板 = web bundle | 满足：workbench loadURL |
| 若有壳转发 snapshot，只透传不改写 | 满足且更强：**无转发**；web 直连后端 |

## 非目标 / 留给兄弟包

- Web 类型、hydrate、PermissionCard、vitest → **WP-184**
- BE snapshot 落盘 → **WP-181**
- 可选：WP-181+web 后桌面打开会话冒烟（本包未跑，重型暂停）

## done_when

- [x] 确认无协议分叉
- [x] 文档记录（本文件）
- [x] 不改 Rust
