# Issue 分级与阻断规则（Batch 1 团队上下文）

版本：v1（2026-10-11，总经理起草，待 Andy 审）。基线提交：`e583ff2`。配套：`knowledge-node-admission.md`、`issue.schema.json`。

## 1. 级别（四级，互斥，取最高可证级别）

| 级别 | 代号 | 定义 | 判定一句话 | 例 |
|---|---|---|---|---|
| 架构错误 | `arch` | 设计本身错：两条主张互相矛盾、出现第二个真源/第二个写者、所有权成环、安全边界不止一处实现。改代码不够，必须改契约。 | “照着知识节点/ADR 一字不差地实现，系统仍然是错的。” | 两个模块各自维护 session 行身份；工具可绕过管线授权直达执行。 |
| 契约漂移 | `drift` | 实现与其契约（enabled 节点、ADR、规范 docs、死亡清单/契约测试）不一致；两者之一要改，但契约本身自洽。 | “代码和它的契约说的不是一件事。” | 文档没跟上代码；死亡清单误报；测试断言过期。 |
| 确认 bug | `bug` | 违反明确承诺的错误行为，且有可复现证据（失败测试、复现命令或可判定的代码路径）。 | “按承诺应 A，实际复现出 B。” | 符号链接本应拒绝复制，实际复制了目标。 |
| 疑似 | `suspect` | 有迹象但证据不足以复现，或只在某平台/环境出现且原因未查清。 | “我怀疑，但还拿不出复现。” | Linux 下 stop 后 worker 仍在，原因未定位。 |

正交字段（不改变级别，只影响处置）：

- `layer`：`product`（产品代码/行为）｜`gate`（守门的测试、死亡清单、文档本身有错）｜`env`（只在某平台/环境出现）。
- `core`：是否触及核心节点（所有 `enabled` 节点 + Batch 2 目录评审时总经理标为核心的节点）。
- `basis`：触及的准入依据 A1–A7。

升级/降级：`suspect` 拿到复现后改为 `bug`/`drift`；`bug` 或 `drift` 查明根因是契约自相矛盾时改为 `arch`。改级别必须追加一条 evidence，不能只改字段。

## 2. 必填字段

`id`（ISSUE-NNN，递增不复用）、`grade`、`layer`、`domain`（board 域 id）、`core`、`basis`、`title`（一句话）、`promise`（被违反的承诺，引用节点键或 ADR/规范编号/测试名）、`actual`（实际行为一句话）、`evidence[]`（每条：`path` + `symbol` 或 `test` 或 `lines`，加 `commit` 短哈希）、`repro`（测试名或命令；`suspect` 可空但需写“缺什么才能复现”）、`reporter`、`owner`、`status`（`open`｜`confirmed`｜`blocked`｜`closed`｜`wontfix`）、`blocks`（是否触达阻断阈值，bool）。机读格式见 `issue.schema.json`。

证据要求：路径+符号/测试名必须在基线提交上可打开；不得以聊天、口述、本地临时文件为证据。运行测试时环境变量与临时目录限定在仓库内：`HOME=/workspace/litecode/.tmp/home … </dev/null`。

## 3. 阻断阈值（任一命中 → 整批阻断）

| 编号 | 条件（可检查） |
|---|---|
| B1 | 存在 `grade=arch` 且 `status=confirmed` 的 issue（确认 = 域负责人 + 总经理各自复核过证据）。 |
| B2 | 任一 `core=true` 的节点在 Batch 3 得出结论 B（无法拓展）。 |
| B3 | 存在 `grade=bug`、`basis` 含 A6、且绕过的是**全局唯一安全边界**的 issue：工作区沙箱/路径解析（`src/workspace/sandbox.rs`）、权限 floor 与授权管线（`src/permission/*`、`src/tool/authorize.rs`）、服务认证与绑定（`src/serve/*`）。工具局部防护（如 webfetch SSRF）不触发 B3，按高优先 issue 处理。 |
| B4 | 同一个 `core` 节点上 `status∈{open,confirmed}` 的 `drift(layer=product)`+`bug` 合计 ≥ 3。 |
| B5 | 全批 `status∈{open,confirmed}`、`core=true`、`layer=product` 的 `drift`+`bug` 合计 ≥ 12。 |

不触发阻断：`suspect`（任何数量）；`layer=gate` 与 `layer=env` 的 issue（它们是守门失效，记录并排入修复，但不说明架构不稳）；非核心节点上的问题。

B4、B5 的数字是首版标定，需 Andy 确认或调整。

## 4. 上报路径

1. **发现者 → 域负责人**：写入 issue（字段齐全），私信负责人 issue id。成员不得直接找 Andy。
2. **域负责人 → 总经理**：负责人复核证据与级别；可能触达 B1–B5 时立即私信总经理，否则在本域工作包收尾时汇总。
3. **总经理 → Andy**：总经理复核；命中阈值时
   - 用 `wp_gate` 把本批所有进行中的工作包置为 `blocked`，`append_log --kind block` 记录命中的阈值编号与 issue id；
   - 只在与 Andy 的 1:1 对话中上报：命中阈值、issue 摘要、证据链、建议的两三个处置选项；
   - 阻断期间成员停止新建节点，只允许补证据。
4. **解除**：只有 Andy 能解除整批阻断；解除决定由总经理 `append_log --kind decision` 记录后恢复工作包。

## 5. 种子 issue（基线 `e583ff2`，Linux 云端测试结果 + 代码阅读）

核实方式：标“已读码”的已打开源码核对；其余沿用测试输出，未单独复现。

| id | 级别 | layer | 域 | core | 摘要 | 证据 | 影响架构稳定性 |
|---|---|---|---|---|---|---|---|
| ISSUE-001 | drift | gate | client-ux | 是（client-projection） | 2 个测试文件编译失败：`InternalEvent::PermissionAsk` 新增 `free_text`/`kind`/`multi_select`，`PendingPermission`/`PermissionRequest` 加字段，测试未跟上。整个测试二进制不运行，客户端权限投影失去守护。 | `tests/client_protocol_project.rs` `fn permission_ask_projects_to_permission_request_with_session_id`（L397–413，已读码）；`src/runtime/observer.rs` `PermissionAsk`；`tests/client_protocol_permission.rs` | 否（守护缺口，修测试即可） |
| ISSUE-002 | drift | gate | ide-substrate | 是（retrieval-contract） | `lexical_lane_source_never_calls_text_index_accelerator` 按字符串断言 `lexical_search_ripgrep(query, preset)`，函数已改为 4 参数。不变量（词法腿走磁盘遍历、不接索引加速）仍成立。 | `src/engines/code_search/lexical.rs` `fn lexical_search_with_preset`（调用 `lexical_search_ripgrep(query, preset, false, None)`）与测试 L833–849（已读码） | 否 |
| ISSUE-003 | drift | product（文档） | harness-core | 是（session-model） | `src/session/DATA.md` 未出现 `SessionEvent` 词汇，而 `src/session/event.rs` 定义了带 `seq`/`surface_op`/`source_seqs` 的 `SessionEvent`；G1 词汇门 `session_seq_g1_envelope_vocab_matches_mental_model` 失败。 | `src/types/death_list_gate.rs` `fn session_seq_g1_envelope_vocab_matches_mental_model`；`src/session/event.rs` `struct SessionEvent`（已读码） | 否（文档漂移；但会话真源词汇是核心，需在 Batch 3 定哪边为准） |
| ISSUE-004 | suspect | product | ide-substrate | 否 | `optional_lsp::agent_lsp_tool_line_text_scenarios`：findReferences 输出缺上下文块。未定位是工具输出整形变更还是测试过期。 | `tests/optional_lsp.rs` `fn agent_lsp_tool_line_text_scenarios` | 否 |
| ISSUE-005 | drift | product | model-config | 否 | `src/config/manager.rs` `fn repair_agent_models_on_boot` 直接持有 `rusqlite::Connection`，在 `src/config/global_db/` 之外触达全局库，违反 SQL 死亡清单“rusqlite 只在 session/data/sqlite 与 config/global_db 内”。 | `src/config/manager.rs` `fn repair_agent_models_on_boot`；`tests/session_data_sql_death_list.rs` `fn is_global_db_ok`（已读码） | 否（局部越界，一处） |
| ISSUE-006 | drift | gate | harness-core | 否 | SQL 死亡清单把 `src/engines/session_search/dense_parity.rs` 判为越界：该文件是 `#[cfg(test)] mod dense_parity;`（文件级测试模块），扫描器只跳过内联 `mod tests {}`。误报。 | `src/engines/session_search/mod.rs` L8–9；`tests/session_data_sql_death_list.rs` `fn production_lines`（已读码） | 否 |
| ISSUE-007 | drift | gate | ide-substrate | 否 | 热切换死亡清单用子串匹配 `reload_workspace`，命中的是排除规则重载 `reload_workspace_excludes_from_disk`（`src/workspace/filter/workspace_excludes.rs`、`src/engines/code_search_ipc/server.rs`）和配置应用计划字段 `plan.reload_workspace`（`src/runtime/mod.rs` `fn apply`），都不是工作区根热切换。误报。题述 `config/gate/mod.rs` 在当前基线未命中。 | `tests/workspace_hotswitch_death_list.rs` `FORBIDDEN`；上述三处（已读码） | 否 |
| ISSUE-008 | drift | gate | client-ux | 否 | `web/src/components/PermissionModal.test.tsx` L260 断言提问选项按钮含 `justify-start`；按钮由 `web/src/components/AskUserCard.tsx` 渲染，`web/src` 生产代码中已无 `justify-start`。样式断言过期。 | 测试 L255–260；`web/src/components/AskUserCard.tsx` L271 附近（已读码） | 否 |
| ISSUE-009 | suspect | env | ide-substrate | 是（engine-lifecycle） | Linux 下 code_search stop 后 worker 进程未被结束。若属实，违反“Stopped 即无运行实体”与子进程寿命绑定。 | `src/proc_lifetime.rs`；`src/engines/code_search_ipc/*`；失败测试见云端测试记录 | **可能**：确认后升 `bug`，计入 B4 |
| ISSUE-010 | bug | product | ide-substrate | 是（workspace-shared-substrate） | `copy_path` 先经 `sandbox.resolve` 解析（跟随符号链接），再由 `copy_recursive` 用 `symlink_metadata` 检查，顶层符号链接已被解析成目标，于是复制了目标文件；子目录内的链接仍被拒。目标仍在工作区内，未越界。该测试只在 unix 编译，Windows 开发机上从未运行。 | `src/workspace/service.rs` `impl WorkspaceService › fn copy_path`、`fn copy_recursive`、测试 `fn refuses_to_copy_symlinks`（已读码） | 否（不越界，不触发 B3） |
| ISSUE-011 | drift | gate | tools-permission | 否 | `ssrf_rejects_missing_host` 前提错误：`url` 按 WHATWG 把 `http:///no-host` 解析为主机 `no-host`，测试结果取决于运行环境 DNS（云端解析到 198.18.0.1 被放行）。 | `src/tools/webfetch.rs` 测试 `fn ssrf_rejects_missing_host`、`fn validate_public_http_url`（已读码） | 否 |
| ISSUE-012 | bug | product | tools-permission | 否 | 由 ISSUE-011 牵出：`validate_public_http_url` 的非公网判定漏掉 IPv4 映射 IPv6（`::ffff:127.0.0.1`、`::ffff:10.0.0.1`）、IPv6 链路本地 `fe80::/10`、`198.18.0.0/15`（基准测试段）等；std 判定已在云端用独立小程序核实这些地址全部判为“公网”。另：校验时解析 DNS、请求时 reqwest 再次解析，存在 DNS 重绑定窗口（此条为 suspect）。 | `src/tools/webfetch.rs` `fn validate_public_http_url`、`fn is_ipv4_shared`、`impl WebFetchTool › fn fetch` | 否（工具局部防护，不触发 B3）；**安全高优先**，建议本批首个修复 |

种子统计：arch 0；bug 2；drift 8（product 2、gate 6）；suspect 2。核心节点上 open 的 product 级 drift+bug：ISSUE-003（session-model）、ISSUE-010（workspace-shared-substrate）= 2，均未达 B4/B5。**当前无阻断。**
