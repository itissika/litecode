# WP-203 实现 — webfetch / websearch 进 Code（软失败）

**分支：** `feat/tool-contract-ptc`（不合 main）  
**口径：** Andy 2026-10-22。排在 WP-202 之后或中层串行不撞车即可。

## 目标

1. `webfetch` / `websearch` → **Code-only**（无 Direct）。
2. **软失败**：HTTP/业务失败应作为工具数据或可 catch 的 `ToolError`，**尽量不打崩整次 `run_code`**；仅协议/沙箱/真正基础设施故障可升为程序失败（方案写清边界）。
3. **TUN / 代理**：Andy 反馈开 TUN 后 fetch 因过度防御不可用——查清并尽量放开合理路径（私网/代理场景），结论写入 dig；能修则修。

## 验收

1. 两工具进 Code；软失败可证（测例）。  
2. TUN/过度防御有 dig 结论或修复。  
3. cargo 绿；artifacts；未合 main；无 FE。

## 非目标

合 main；改 libs；LSP。
