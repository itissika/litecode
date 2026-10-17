# WP-189 验收说明

> owner：tools_steward · approver：gm · tools_version：**24**  
> 日期：2026-10-16（Asia/Hong_Kong）

## 1. 交付摘要

在现有 `.litecode/ops` / `wp.mjs` 上增量落地组织树推导、role charter、向上请求闭环、关键 CLI 中文报错；**不**迁 `.agent-org`、不削弱方案审批≠完成验收、不引入 hourly/巡逻。

| 步 | tools_version | 内容 |
|---|---|---|
| 1 | 22 | `org_nodes`/`ops_config`/agent 挂树；`deriveReporting`；validate 漂移中文；`org-tree`/`org-sync` |
| 2 | 23 | `role-charter`；`context/constitution.md` 占位 |
| 3 | 24 | `request`/`handle`/`forward`；`kind`/`requester`；`team_ops.request_resolved`；中文 CLI 错误 |

## 2. handle 最终语义（覆盖原 plan §3.2）

原 plan §3.2 写「handle → intake_agent」。**GM 批准额外口径覆盖如下：**

```text
handle --id WP-x [--owner AGENT | --intake]
```

| 写法 | owner 结果 | 适用 |
|---|---|---|
| （无 flag） | **当前 approver 自留** | 域内向上请求默认 |
| `--owner AGENT` | 指定已登记 agent | 中层指定 session/tools 等执行 |
| `--intake` | `ops_config.intake_agent`（默认 tools_steward） | **仅 ops/工具类**场景 |

禁止把域内向上请求一律砸给 tools_steward。演示按「backend_mid 自留 → `--owner session`」展示。

## 3. done_when 对照

| done_when | 证据 |
|---|---|
| 组织树可推导汇报线且 validate 能抓住手填冲突 | `org-tree`：gm←andy、session←backend_mid；故意改 `session.superior=gm` → validate 中文失败；`org-sync` 后通过。单测 `org_drift` |
| role charter 对现有角色可生成提示词 | `role-charter --id` gm/backend_mid/session/tools_steward 含上级与 accept/reject。单测 `charter_smoke` |
| request/handle/forward 可演示一条向上请求闭环 | 见 §4 实板 WP-190；单测 `request_loop`（含 forward 到 gm、禁上交 andy） |
| 关键 CLI 错误为中文且含当前状态 | 单测 `zh_errors`（≥5 条：fill/submit/未知身份/approve/request） |
| 现有 wp 生命周期与重任务闸门/事件驱动未被削弱 | `wp.test.mjs` 既有组全绿；未改 `init_env`/内存看门狗；未新增 hourly；`DELIVER_TYPES` 仅追加 |
| 有验收说明与回归测或手工步骤 | 本文件 + `demo-transcript.txt` |

## 4. 实板演示（WP-190）

命令抄录见 `.litecode/ops/artifacts/WP-189/demo-transcript.txt`。摘要：

```text
session        request   → WP-190 awaiting_approval，approver=backend_mid
backend_mid    approve
backend_mid    handle                # mode=keep，owner→backend_mid
backend_mid    handle --owner session
session        start → deliver
backend_mid    close --to done --note '…'
→ team_ops.request_resolved target=session（inbox+outbox）
```

history 含：create / submit / approve / handle / handle / start / deliver / close。

## 5. 回归命令与结果

```bash
node --test .litecode/ops/tools/wp.test.mjs
# （本包测试 runner 为自研 group，实际：）
node .litecode/ops/tools/wp.test.mjs
# → 16 groups, 217 checks, ok

node .litecode/ops/tools/validate_all.mjs
# → ops board OK；kb errors=0
```

抽查：`wp summary --id WP-188` / `history --id WP-188` 不炸（旧包无 kind）。

## 6. 改动文件（ops 路径）

- `.litecode/ops/board.schema.json` — org_nodes / ops_config / agent.node|superior|subordinates / WP.kind|requester / history handle|forward
- `.litecode/ops/board.json` — 迁移 13 agents 挂树 + org_nodes + ops_config
- `.litecode/ops/validate.mjs` — orgProblems + request requester
- `.litecode/ops/tools/_lib.mjs` — deriveReporting / charterOf / DELIVER_TYPES+=request_resolved
- `.litecode/ops/tools/wp.mjs` — 新命令 + 中文错误 + handle 口径
- `.litecode/ops/tools/wp.test.mjs` — 断言中文化 + org_drift/charter_smoke/request_loop/zh_errors
- `.litecode/ops/tools/registry.json` — tools_version=24；ops.wp v7；validate_board v10
- `.litecode/ops/tools/board.md` — 版本头与变更摘要
- `.litecode/ops/context/constitution.md` — 占位
- `.litecode/ops/artifacts/WP-189/{acceptance,demo-transcript,plan,brief}.md`

## 7. 非目标确认

- ❌ 未迁 `.agent-org` / 未引入 aorg CLI  
- ❌ 未重编号历史 WP  
- ❌ 未改产品 Rust/web 代码、未 commit/push、未碰 main  
- ❌ 未削弱 done 门禁 / 未合并「方案批=验收」  
- ❌ 未引入 hourly / 常驻巡逻新路径  

## 8. Blocker

无。
