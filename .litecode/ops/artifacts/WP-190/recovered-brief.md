# WP-189 — ops 工具易用度优化（借鉴 agent-org）

## 方向（GM）

在现有 `.litecode/ops` / `wp.mjs` 上借鉴本地优化包 `agent-org` 的易用点，**不整包替换**。

参考只读副本：`/workspace/ops-pkg-review/agent-org/`（及 `sim/` 实跑记录）。

## 必须落地

1. **组织树推导汇报线**：上下级 / 审批人从组织结构自动算出；手填与树冲突时 `validate` 能抓住。
2. **role charter**：按角色生成岗位提示词，可供中层/执行者粘贴使用。
3. **向上请求**：`request` / `handle` / `forward`（或等价命令）可演示一条「下级 → 上级」闭环。
4. **中文报错**：关键 CLI 错误直白中文，并带上当前状态。

## 必须保留（不可削弱）

- 方案审批与完成验收分离
- 只靠事件驱动（不要引入 hourly / 常驻巡逻）
- 重任务闸门（≥2GiB、同时仅一个重任务）
- 现有 webhook / artifacts / history 契约（可扩展，不无故改名换头）

## 非目标

- 不引入整包 aorg / `.agent-org` 目录迁移
- 不把 188 历史 WP 改编号重灌
- 不借机改 bot 身份认证大模型（A1 冒充若顺手加固可记笔记，不阻塞本包）
- 不带入游戏 Unity 白名单脚本

## 验收（done_when）

见看板 `done_when`。方案由工具集管家 fill/submit，GM 批方案后再实现。
