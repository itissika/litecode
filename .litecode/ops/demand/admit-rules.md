# 需求升管线 / 拒绝规则（需求官）

路径：`.litecode/ops/demand/`
更新：2026-10-11（WP-003）

## 可升管线（intake → refined → scheduled → in_pipeline）

须同时满足：

1. **有域**：`domain` 对应 `ops/board.json` 已有域，或已由总经理立域 WP。
2. **可调度验收**：`acceptance[]` 至少 1 条，且每条能用路径 / 命令输出 / 面板状态客观判定（是/否）。
3. **来源可追溯**：`source` 写明 Andy 消息、WP、或事件 id。
4. **不踩断点**：不要求在 Andy 审过 + 群启动令前建产品 bot、发产品首条、或开产品域并行 WP。
5. **本机边界**：产物落本机 litecode 树；不要求在云端写 Andy 相关状态；开源仓不含个人信息。

炼制完成（验收可调度）→ `status=refined`；总经理排入 WP 后 → `scheduled` / `in_pipeline`。

## 直接拒绝（rejected）

任一即拒：

- 无 `acceptance`，或验收不可检测（纯口号、无证据位）
- 要求跳过验收标准直接进管线 / 直接改产品代码
- 要求改 `status: enabled` 知识节点且无 Andy 批准
- 要求群内讨论（团队群仅 Andy 公告）或代 Andy 发言
- 要求破坏工具 gate（去掉 done 门禁、破坏性原地改契约）
- 与现行断点冲突（WP-001/002：审过前不建产品 bot、不发产品首条）

## 需求官边界

- 只维护 demand 板与验收炼制；不改产品代码；强制走 ops.tick / validate / append_log。
- 团队规模扩张须经总经理 WP，不由需求官单独升管。
