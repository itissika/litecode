```node
node : process-separation
status : pending
summary : 内核独立进程：本地 sidecar / 远程 SSH；serve 单端口收敛，切换只动传输层。
x : 34
y : 1044
```

**内核始终是独立进程；位置（本机 / 远程）只改变取得与连接方式。**

- serve 单端口收敛：一条路由表挂 `/ws` + `/api/workspace` + `/api/settings` 等，入口级 tracing [@ file="src/serve/router.rs" label="router.rs"]。
- Electron 宿主：spawn sidecar（Rust server，就绪握手）；Home hub 提供 Local / Remote（SSH）向导 [@ file="desktop/src/sidecar.ts" label="sidecar.ts"] [@ file="desktop/src/hub.ts" label="hub.ts"]。
- 一进程一 workspace：切换工作区＝杀旧起新 [@ file="desktop/src/main.ts" label="main.ts"]。
- 浏览器形态：同一 web UI 由 sidecar 托管（开发场景）。
- 独占约束：工作区被跨进程锁持有，一个工作区同时只有一个持有者 [@ file="src/session/workspace_lock.rs" label="workspace_lock.rs"]。


