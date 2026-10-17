# WP-189 方案：ops 工具易用度优化（借鉴 agent-org）

> 状态：方案（待 GM 批方向与验收口径）  
> owner：tools_steward · approver：gm  
> 当前基线：`tools_version=21`（以 `registry.json` 为准）  
> 原则：**在现有 `.litecode/ops` / `wp.mjs` 上增量借鉴，不整包替换、不迁 `.agent-org`。**

---

## 1. 目标与非目标

### 目标（对应 brief / done_when）

1. **组织树推导汇报线**：上下级 / 默认审批人从组织结构算出；手填与树冲突时 `validate` 失败并指出差在哪。
2. **role charter**：按角色（board.agents）生成可粘贴的岗位提示词（含上下级、职责边界、总章程摘要）。
3. **向上请求闭环**：`request` →（可选 `forward`）→ `approve` → `handle` → 交付/关闭，可演示一条「下级 → 上级」链路；事件可投递、可 ack。
4. **关键 CLI 错误中文化**：`wp.mjs`（及同源写路径）关键失败信息改为直白中文，并带上**当前状态**（如 `当前 drafting`）。

### 必须保留（不可削弱）

| 项 | 口径 |
|---|---|
| 方案审批 ≠ 完成验收 | 现有 `submit/approve` 与 `deliver/close(done)` 分轨不变；请求包也不得跳过验收门禁（除非显式配置，本包默认不引入「免验收」） |
| 事件驱动 | 不引入 hourly / 常驻巡逻新路径；沿用现有 inbox/outbox + `wp_watch` 心跳模型 |
| 重任务闸门 | ≥2GiB 启动门槛、同时仅一个重任务等全局规则不动；本包不改 `init_env` / 内存看门狗 |
| webhook / artifacts / history | 现有事件类型、产物目录、history 字段**可扩展**；不无故改名换头 |

### 非目标

- 不引入整包 aorg、不迁数据到 `.agent-org/`
- 不重灌 / 改号历史 WP（WP-001…188 保持原样）
- 不带 Unity 白名单、不改 bot 身份认证大模型（A1 冒充若顺手加固只记笔记）
- 本包**只出方案阶段不做产品代码实现**；实现阶段仍禁止破坏性原地拆契约（须新 tool id / 版本递增）

---

## 2. 借鉴点映射（agent-org → litecode ops）

| # | agent-org | litecode 落地 | 不照搬 |
|---|---|---|---|
| A | `org/nodes` + `derive()`：成员上级=节点 lead，lead 上级=父节点 lead，根 lead 上级=principal | 在 board 旁增加轻量 `org_nodes`；`_lib.mjs` 提供 `deriveReporting(board)`；可选把推导结果缓存到 agent 字段 | 不拆独立 `.agent-org` 目录树；不强制每人一张独立 role json 文件 |
| B | `derivedDrift` + `aorg validate` | `validate.mjs` / `validate_all` 增补：手填 `superior`/`subordinates` 与推导不一致 → INVALID | 不把 org sync 做成静默覆盖生产 board 的默认写路径（sync 显式命令） |
| C | `aorg role charter` | `wp.mjs role-charter --id <agent>`（或等价子命令）渲染 markdown | 不要求先 rewrite agents 字段名为 duties/does；映射现有 `scope`/`tools`/`accept_tests`/`reject_tests`/`place` |
| D | `wp request` / `handle` / `forward` + `requester` + `kind=request` | 扩展 `ops.wp`：**新增**子命令与可选 WP 字段；生命周期仍走 awaiting_approval → ready → … | 不替换现有 `new/fill/submit`；请求包编号继续 `WP-NNN`，不另起序列 |
| E | `OrgError` 中文 +「当前 status」 | 统一 `dieZh(cmd, id, msg, status)` 风格改关键错误 | 不改 JSON stdout 成功契约的英文字段名（`ok`/`cmd`/`status` 等保持） |
| F | `config.roles.intake` | board 级可选配置（如 `board.ops_config.intake_agent`，默认 `tools_steward`）供 `handle` 转交 | 不复活已撤销的 demand board |
| G | `request_resolved` 事件 | 新增可投递类型 `team_ops.request_resolved`（扩展 `DELIVER_TYPES`），不改旧类型语义 | 不改现有 HMAC/webhook 头名（若有 litecode 侧签名约定则沿用） |
| H | `aorg org sync` 写回 superior | `wp.mjs org-sync --as gm|andy` 显式重写缓存字段 | 禁止 tick/hourly 自动 sync |

参考只读：`/workspace/ops-pkg-review/agent-org/`（`docs/concepts.md`、`docs/cli.md`、`src/org/graph.mjs`、`src/wp/actions.mjs`、`src/org/mutate.mjs#charterOf`）与 `sim/.agent-org/` 实跑样例。

---

## 3. 设计

### 3.1 数据模型

#### 3.1.1 组织节点（新增，挂在 board 根上）

```json
"org_nodes": [
  { "id": "litecode", "name": "LiteCode", "parent": null, "lead": "gm", "goal": "产品交付与团队运转" },
  { "id": "backend", "name": "后端", "parent": "litecode", "lead": "backend_mid", "goal": "后端域统筹" },
  { "id": "frontend", "name": "前端", "parent": "litecode", "lead": "frontend_mid", "goal": "前端域统筹" },
  { "id": "ops", "name": "工具集", "parent": "litecode", "lead": "tools_steward", "goal": "强制工具集与热更" }
]
```

- `lead` 必须是已登记 agent（或后续明确允许的特殊 id）；同一 lead 不得挂两个节点。
- **委托人 principal**：固定为 `andy`（已在 `--as` 白名单，但不是 `agents[]` 成员）。根节点 lead（gm）的推导上级 = `andy`。

#### 3.1.2 Agent 可选字段（schema 扩展，`additionalProperties` 仍 false，显式列出）

| 字段 | 含义 | 谁写 |
|---|---|---|
| `node` | 所属组织节点 id | 人填 / 初始化脚本 |
| `superior` | 直接上级（agent id 或 `andy`） | **仅** `org-sync` 或人填；validate 对漂移报错 |
| `subordinates` | 直接下级 id 列表 | 同上 |

推导规则（对齐 agent-org `derive`）：

1. 若 `agent.id === node.lead`：上级 = 父节点 lead；无父则 = principal(`andy`)。
2. 否则：上级 = 本节点 `lead`。
3. `subordinates` = 所有「推导上级 = 自己」的 agent id，排序稳定。

**首迁映射（实现时落一版，可在 plan 批准后微调，但须在验收文档写死）：**

| agent | node | 备注 |
|---|---|---|
| gm | litecode（lead） | 上级 andy |
| backend_mid | backend（lead） | 上级 gm |
| frontend_mid | frontend（lead） | 上级 gm |
| tools_steward | ops（lead） | 上级 gm；承接 handle 默认 intake |
| session, tools, runtime, context, ide, provider, demand_officer | backend | 上级 backend_mid |
| frontend, frontend_desktop | frontend | 上级 frontend_mid |

未挂 `node` 的 agent：validate **警告或 error**（本包建议 **error**，避免半套树上线）；实现前用一次性迁移把上表写齐。

#### 3.1.3 工作包可选字段

| 字段 | 类型 | 说明 |
|---|---|---|
| `kind` | `direct` \| `split` \| `request` \| 缺省 | 缺省/旧包视为普通包；`content.kind` 仍只服务方案正文（direct/split），**不**与 WP.kind 混用 |
| `requester` | string \| null | 仅 `kind=request`；发起向上请求的人 |

`history.action` 枚举**扩展**（不改旧值）：`handle`、`forward`（`request` 动作用现有 `create`+`submit` 记录即可）。

`board.ops_config`（可选对象，一次加好避免多次破 schema）：

```json
"ops_config": {
  "principal": "andy",
  "intake_agent": "tools_steward",
  "escalate_to_principal": false
}
```

- `escalate_to_principal: false`：`forward` 不得把审批人改成 `andy`（与 agent-org 默认一致；GM 是顶）。

### 3.2 新命令（均 `--as` 自报；加性变更，bump `ops.wp` version）

| 命令 | 谁可调 | 行为摘要 |
|---|---|---|
| `org-tree [--json]` | 只读，可无 `--as` | 打印节点缩进树 + lead + 成员 |
| `org-sync` | gm / andy | 按树重写所有 agent 的 `superior`/`subordinates`；写 board + validate + history/log |
| `role-charter --id <agent>` | 只读 | stdout 打印 markdown 岗位说明 |
| `request --title T --goal G [--domain D] [--text\|--content-file]` | 非 principal | 新建 `kind=request` 包：owner=requester=`--as`，approver=推导上级，status=`awaiting_approval`，自动带方案正文（goal/text），emit `team_ops.approval_requested` |
| `handle --id WP-x` | 当前 approver | 仅 `kind=request` 且 status=`ready`：owner → `intake_agent`（若配置且存在）否则 approver 自己；记 history `handle` |
| `forward --id WP-x [--note N]` | 当前 approver | 仅 `awaiting_approval`：approver → 自己的上级；触发新的 `approval_requested`；记 `forward` |

**闭环演示（验收脚本）：**

```text
session  --as→ request
backend_mid     approve
backend_mid     handle   （owner → tools_steward）
tools_steward   start → deliver
backend_mid     close --to done --note '…'   → emit team_ops.request_resolved → requester
```

可选分支：`forward` 在 approve 前上交到 gm。

### 3.3 validate 规则（新增）

1. `org_nodes`：id 唯一、parent 存在/非自指/无环、lead 唯一且指向已知 agent、lead 的 `node` 必须等于该节点（若已填 node）。
2. 每个 active agent 必须有合法 `node`（本包强制）。
3. **漂移**：若存在手填 `superior`/`subordinates`，必须与 `deriveReporting` 一致；文案中文，例如：`角色 session 的 superior 应为 backend_mid，当前是 gm`。
4. `kind=request`：必须有 `requester`；`requester` 必须已知；关闭 done 时建议强制 note（与 agent-org 对齐，写入 wp close 逻辑）。
5. 旧 WP 无 `kind`：行为不变。
6. **不**放宽：父包未批子包不能 submit/start；done 仅 review + approver；子包未封父包不能 deliver/close。

### 3.4 错误文案策略

- 覆盖面：身份未知、状态不允许、缺方案、父包未批、子包未封、校验回滚等**关键用户可见**错误。
- 模板：`<动作> <WP-id>：<原因>（当前 <status>）`；无 WP 时带命令名。
- 成功路径 JSON 字段名保持英文，避免破现有解析。
- 单测：对若干 `die` 路径断言 stderr 含中文与 `当前`。

### 3.5 契约与版本

- **破坏性**（改旧命令语义 / 删字段 / 改事件名）→ **禁止**；若未来需要，新 tool id。
- 本包：`ops.wp` version++、`ops.validate_board` version++；`tools_version` 按实现步骤递增（见下）。
- `DELIVER_TYPES` 仅**追加** `team_ops.request_resolved`。
- artifacts 仍落 `.litecode/ops/artifacts/WP-xxx/`；请求包可把演示笔记录进本包 artifacts。

### 3.6 charter 渲染结构（建议）

```markdown
# 岗位说明：{name}（{id}）
状态 / 所属节点 / 上级 / 下级
## 职责（scope）
## 工作地点（place）
## 工具
## 接受的交付（accept_tests）
## 不接受的交付（reject_tests）
## 总章程
（读 `.litecode/ops/context/constitution.md`；若无则生成最小占位并由 gm 后续补，不阻塞命令）
```

---

## 4. 实现步骤（小步 · tools_version 递增）

> 实现阶段才改代码；本方案批准前不落地。每步：改工具 → 测 → `tools_version++` → 更新 `registry.json` purpose / `board.md` 变更摘要。

| 步 | 内容 | 预期 tools_version |
|---|---|---|
| 0 | （本包）方案 fill/submit，GM 批准 | 21（不变） |
| 1 | schema：`org_nodes`、agent 可选字段、`ops_config`；`deriveReporting` + validate 漂移；迁移现有 13 agents 挂树；`org-tree` / `org-sync` | **22** |
| 2 | `role-charter` + 最小 `constitution.md` 占位；单测快照含上级 | **23** |
| 3 | `request` / `handle` / `forward` + WP `kind`/`requester` + history 动作 + `team_ops.request_resolved`；演示脚本 / 单测闭环 | **24** |
| 4 | 关键 CLI 错误中文化（可与步 3 同 PR，若拆分则 **25**） | **24 或 25** |
| 5 | 验收说明 `acceptance.md`、回归清单；`deliver` → GM `close` | （实现末版） |

闸门自检（每步）：`node .litecode/ops/tools/validate_all.mjs`；`node --test .litecode/ops/tools/wp.test.mjs`（及新增用例）。

---

## 5. 验收与回归（对齐 done_when）

| done_when | 怎么证 |
|---|---|
| 组织树可推导汇报线且 validate 能抓住手填冲突 | ① `org-tree` 显示 gm←andy、session←backend_mid 等；② 故意把 `session.superior` 改成错误值 → validate 失败且中文指出期望/实际；③ `org-sync` 后通过 |
| role charter 对现有角色可生成提示词 | 对 `gm` / `backend_mid` / `session` / `tools_steward` 各生成一份，含上级与 accept/reject |
| request/handle/forward 可演示一条向上请求闭环 | 按 §3.2 演示（或 forward 分支）；board history 含 submit/approve/handle/…；requester 收到 `request_resolved`（inbox/outbox 可见） |
| 关键 CLI 错误为中文且含当前状态 | 对 `fill` 在非 drafting、`submit` 缺 content、错误 `--as` 等至少 5 条断言 |
| 现有 wp 生命周期与重任务闸门/事件驱动未被削弱 | 回归：旧 WP 无 kind 的 approve/deliver/close 测例全绿；确认未新增 hourly 入口；内存相关脚本未改 |
| 有验收说明与回归测或手工步骤 | `artifacts/WP-189/acceptance.md` + 测试或手工命令抄录 |

**回归最小集（实现后）：**

- `wp.test.mjs` 现有用例
- 新：`org_drift`、`charter_smoke`、`request_loop`、`zh_errors`
- `validate_all` OK
- 抽查：任意旧 `WP-188` 只读 `summary/history` 不炸

---

## 6. 风险与不做清单

### 风险

| 风险 | 缓解 |
|---|---|
| schema 一次加字段导致旧 board 不兼容 | 新字段一律 optional；迁移脚本与 org-sync 同步合入 |
| 请求包与普通包审批人语义混淆 | `kind=request` 分支显式判断；默认审批人=推导上级，禁止手填漂出树（validate） |
| 事件类型膨胀导致 webhook 消费者未知 | 只追加一种；旧消费者忽略未知 type 的行为保持 |
| 中文错误影响依赖英文 stderr 的脚本 | 成功仍走 JSON stdout；仅失败 stderr；在 acceptance 注明 |
| `--as` 仍自报（身份冒充） | 非本包范围；笔记记入 `artifacts/WP-189/notes-a1-auth.md`（可选，不阻塞） |

### 不做清单

- ❌ 迁 `.agent-org` / 引入 `aorg` CLI 二进制依赖  
- ❌ 重编号历史 WP / 重灌 188  
- ❌ Unity 白名单、`aorg-hourly-check`、常驻巡逻  
- ❌ 削弱 done 门禁、合并「方案批=验收」  
- ❌ 改产品 Rust/web 代码（本包仅 ops 工具）  
- ❌ 本方案阶段 commit / push / 改实现代码  

---

## 7. 请 GM 批的方向点（实现细节可后调）

1. 组织树挂在 **board.org_nodes**（而非独立 `.litecode/ops/org/` 目录）——同意？  
2. principal=`andy`，`escalate_to_principal=false`，intake 默认 `tools_steward`——同意？  
3. 首迁节点划分（backend/frontend/ops）如上表——同意或点名调整？  
4. tools_version 分 22→24（或 25）小步递增——同意？

批准后按 §4 实现；本文件即 fill 正文。
