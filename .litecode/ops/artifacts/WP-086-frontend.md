# WP-088 方案（frontend_mid · split）

- 父包：WP-086（GM，approver andy）。并行包：WP-087（backend_mid）。
- 审批：gm。产物：本文件 + 子包产物 `WP-086-web.md`。
- 调研依据：WP-084 前端回报（A5/B3/B5/C3）。
- 中层只拆活、交叉审、汇总。**GM 未批 plan 不 start；不 commit。**

## Goal
子会话前端补齐「查看 / 停止 / 删除」三种操作，发消息永不开放。同时修正删除后的过时显示和重连后的状态。

## GM 裁定
1. 子会话只能查看、停止、删除，**不能发消息**。子会话视图维持现状、不加输入框（`AgentChatInput.tsx:576-616` 是 subagent 分支）。
2. 停止：子会话视图底栏加一个按钮，Workers 名单每行也加一个按钮。
3. 删除：Workers 名单每行加入口（子会话视图可同时加）。子会话在跑时置灰，删除前弹确认框。
4. 删除后的显示：
   - 父会话 tool card 显示「已删除」，不能点；
   - Workers 名单和计数排除该子会话；
   - `subagent-<id>` 面板关闭。
5. 重连：进入 connected 时，在一个统一位置拉一次 session list；`running_with_subagent` 状态要保留，不能丢。

## 已确认协议（backend_mid）
- **(1) 停止**：调 `agent/cancel {session_id: child}`，后端返回真实结果。`cancelled:false` 表示当时没有 turn 在跑：前端不能假定成功，显示中性 toast 或不做处理。
- **(2) 删除**：调 `session/delete {id: child}`。
  - 子会话或其任意后代在跑时，后端拒绝。
  - 删父会话会级联删子会话。
  - lifecycle `deleted` 事件带 `parent_session_id`。
  - 快照的 `subagent_bindings` 对已删的子会话保留墓碑 `deleted:true`：名单和计数排除它，tool card 显示「已删除」。
  - **错误码：占位，待 `WP-086-backend.md` 给出。** 前端先映射已知的"在跑中拒绝"，未知错误一律走通用提示。
  - **墓碑在 wire 上的形状占位，待 backend_mid 给出**：现在类型是 `subagent_bindings?: Record<string,string>`（`web/src/api/types.ts:303`），墓碑可能改成对象值或另开 map。前端要兼容新旧两种写法。
- **(3) 状态**：lifecycle 的 `turn_started` / `turn_finished` 带原始 `status`（含 `running_with_subagent`）。前端直接用它，不再写死 running / idle。
- **(4) 发消息**：后端用专用错误码拒绝对子会话调 `agent/run` 和 enqueue。前端不提供入口，可选择把该错误码映射成提示文案。

## 后端依赖
- **WP-087（backend_mid）**：上面 (1)–(4) 落地、错误码表、墓碑形状。
- 前端可以先按占位开发，用 mock 写测试。**联调验收**等 WP-087 完成后再做。

## 子包
| 子包 | owner | 域 | 产物 |
|------|-------|----|------|
| **WP-089** | `frontend`（前端-web） | client-ux-web | `WP-086-web.md` |

## web 落点（域 bot 实施；行号以调研时为准）
- **停止**
  - 子会话视图底栏：`AgentChatInput.tsx:576-616`（subagent 分支）加"停止"按钮。只在子会话 running / stopping 时可用，调用复用 `turnStore.ts:638` 的 `agent/cancel`。
  - Workers 名单每行：`SubagentRosterPanel.tsx:221-275` 行内加"停止"，不能改变"点行即跳转"的原有行为（`:109-110`）。
  - 返回 `cancelled:false` 时显示中性 toast，不要假装成功。
- **删除**
  - Workers 名单每行加删除入口，可选在子会话视图也加。
  - 子会话在跑时置灰（判断口径同 `SessionItem.tsx:58-60`）；点删除先弹确认框，再调 `session/delete`（复用 `sessionStore.ts:555-581` 的逻辑，或抽出公共函数）。
  - 后端拒绝时按错误码提示；错误码未知时提示通用文案。
- **删除后的显示**
  - `sessionStore.ts:388-395` 的 deleted 分支里，除了现在关的 `agent-<id>`，**也要关 `subagent-<id>`**（这类面板由 `sessionPanelNav.ts:131-139` 打开）。
  - 墓碑数据：`messageStore.ts:375-378,391-399,415-430` 合并 `subagent_bindings` 时保留"已删除"标记。前端额外记录"已删除 id"：收到 deleted 事件且带 `parent_session_id` 时，立即标记到父会话上。
  - tool card：`lib/subagentUi.ts:37-55` 的 `childStatusWord` 遇到已删除返回「已删除」，不能再退回成 "accepted"。`SubagentInlineLine.tsx:37-49` 遇到已删除不渲染成可点按钮。
  - Workers 名单：`SubagentRosterPanel.tsx:168-185` 的临时兜底分支要排除已删除 id，消除"幽灵行"。
  - 计数：`SessionStatusLine.tsx:169-175` 的 total / running 也排除已删除 id。
- **重连**
  - `connectionStore.ts:157-168` 在进入 connected 时**统一**调一次 `listSessions()`。
  - `SessionList.tsx:37-41` 和 `SubagentRosterPanel.tsx:128-135` 打开时各拉一次的逻辑保留（只是打开时刷新），或去重，由实施者判断。不能出现 connected 时重复请求同一个列表的竞态。
- **状态**
  - `sessionStore.ts:380-510` 的 lifecycle `turn_started` / `turn_finished` 分支，有 `status` 字段时用它，没有时回退到原有逻辑。`turn_updated` 的 cancelling → stopping 逻辑保持不变。
- **类型**
  - `web/src/api/types.ts`：
    - `SessionLifecycle` 加 `status?`；
    - `subagent_bindings` 改成兼容墓碑的类型；
    - 新增错误码常量，作为占位。

## 测试清单（vitest）
1. 子会话视图：不渲染输入框和发送按钮（回归）；停止按钮只在 running / stopping 时可用；点击后调 `agent/cancel {session_id: child}`。
2. 停止返回 `cancelled:false`：显示中性 toast，不改状态。
3. Workers 名单每行：停止和删除按钮存在；在跑时删除置灰；点删除先弹确认框，取消后不发 RPC，确认后发 `session/delete`。
4. 删除被拒：已知错误码显示对应提示；未知错误码显示通用提示；列表回滚。
5. deleted 事件（带 `parent_session_id`）到达后：`subagent-<id>` 和 `agent-<id>` 面板都关闭；tool card 显示「已删除」且不可点；名单和计数排除该子会话。
6. 快照里 `subagent_bindings` 带墓碑：重新加载后表现同第 5 条，新旧两种形状都兼容。
7. 重连：进入 connected 时 `listSessions` 只调用一次，不依赖 `SessionList` 是否挂载；`running_with_subagent` 不会被覆盖成 idle。
8. lifecycle `turn_finished` 带 `status:"running_with_subagent"` 时，状态保持；不带 `status` 时回退为 idle。
9. 回归：主会话的取消和删除行为不变；审批卡（AskCard / PermissionModal）不受影响。

## desktop
desktop 内嵌 web，外壳里没有 subagent / delete / cancel 相关代码（`rg desktop/src` 无命中），**预计不用改**。中层只做静态核对（看代码和构建产物）；**真机点击验证由 Andy 做**。

## 不在范围
- 子会话发消息、输入框（GM 裁定永不开放）。
- 后端：WP-087 负责 cancel / delete 的放行与拒绝、级联、墓碑、lifecycle status、`agent/run` 拒绝。
- 归档（archive）、批量删除、父会话级联删除的数量提示 UI：未裁定，不做。
- 不 commit。

## 验收标准
- 测试清单 9 条全绿；`tsc -b` 0 错误；lint 不适用（web 未配置 lint 工具）。
- 和 WP-087 联调通过：子会话停止 / 删除端到端可用，在跑时删除被拒，级联删除后显示正确，重连后状态正确。
- desktop 静态核对通过；禁区没碰；没 commit。

## 档位（来自 WP-084）
- B3「幽灵行 / 计数虚高」：4（常见 bug），本包修。
- B3「`subagent-<id>` 面板残留、tool card 显示 accepted 还能点」：5（边角 bug），本包修。
- B5「重连依赖侧边栏挂载、`running_with_subagent` 丢失」：5（边角 bug），本包修。
- A5「停止 / 删除入口」：1（产品行为待定），已由 GM 裁定。

## 前后端静态 e2e（frontend_mid，2026-10-13 HKT；WP-087 done 之后）
| 项 | Rust | web | 结论 |
|---|---|---|---|
| subagent_binding_states `{call_id:{child_session_id,deleted}}` | protocol.rs:339-349（BufferLoaded）；snapshot 额外插入 connection.rs:946-952；buffer/load connection.rs:1416-1424 | types.ts:305-312,514；messageStore 合并 | 一致 |
| lifecycle deleted `{event,session_id,parent_session_id}` | project.rs:737-746（根会话时为 null）；delete.rs:65,92,124 | sessionStore deleted 分支 | 一致 |
| turn_started/turn_finished.status | status_track.rs:33-48 注入 `as_str()`；manager.rs:121-127 枚举 | types.ts:369-373 四个字符串 | 一致 |
| status_changed | status_track.rs:34-35；project.rs:884-887 | types.ts:413，sessionStore 分支 | 一致 |
| 错误码 | protocol.rs:70,77,79；structured_error_response connection.rs:62-82（data = StructuredError{code,message,retryable} ∪ extra），descendant extra 为 `running_session_ids`（connection.rs:845-849） | agentWs 把 `rpc.error.data` 挂到 err.data；sessionErrors.ts:7-18 读 data.code / data.running_session_ids；types.ts:315-317 | 一致 |
| agent/cancel 回包 | connection.rs:463 `{"cancelled": bool}` | stopSubagent 判断 `cancelled===false` | 一致 |

- cargo：`cargo test --lib -- wp091 status_track session::delete subagent_session_readonly binding_states` → 17 passed。
- web（/exec-daemon/node v22.14.0）：SubagentOps.wp089 / SubagentRosterPanel / SessionStatusLine / sessionStore / connectionStore / messageStore 相关测试 → 5 files / 148 passed；`tsc -b` 0；lint N/A。
- desktop：`git status desktop` 无改动；跑的是同一套 web/dist（main.ts safeLoadURL 加载 sidecar workbench）。**只做了静态核对，真机由 Andy 验。**
