# WP-146（WP-145 后端）：code_search 可见性只看 bind；运行状态只进调用结果

依据：artifacts/bind-warming-research-backend.md、.tmp/gm_bind_warming.md；Andy 已批方向（GM 10-14）。

## 契约 v1（先定，前端 WP-147 与 Tools 照此）
### A. 可见性
- code_search 列入 LLM 工具表 ⇔ 该 agent 的 bind（agent_tools.enabled）为 true。与 engines.json desired、引擎 state、call_gate、索引进度全部无关。
- session_search 不改（保持现有门禁）。
- 运行状态（Warming/Warm/Failed/Idle、重建、job.json）永不写 agent_tools 或 engines.json desired；desired 仍只由人改。
- 不新增状态、事件、刷新机制；回合开始时算工具表的现状保留（bind 只在设置写入时变，已有 reload 路径）。

### B. 默认绑定
- 主 agent（primary 角色，含 default）默认 code_search enabled=true：新库 seed 与已有库迁移各一次（迁移只补“缺行”，已有行无论 true/false 一律不动——人关掉的保持关）。
- 子代理角色不默认绑定。

### C. 调用结果（ToolCallResult::ok，非 error；首行恒为 sync_pct v2 行，正则不变）
| 情形 | 判定 | 首行 | 正文 |
|---|---|---|---|
| 引擎未开 | desired=false | `sync_pct: unknown (engine off)` | `代码引擎未开启，无结果。可在设置中开启代码检索引擎。` |
| 预热中 | desired=true 且 不可搜（state≠Warm 或 call_gate=Wait） | 有精确总数：`sync_pct: N% (warming) chunks d/t`；否则 `sync_pct: scanning (warming)` | `引擎预热中，暂无结果，请稍后重试。` |
| 失败 | state=Failed 或 call_gate=Failed | `sync_pct: failed (<简短原因>)` | 原因一句 |
| 就绪 | is_searchable | 照旧 | 照旧（refreshing 期间照旧可搜，首行带真实 %） |
- 非就绪三种情形一律不返回任何命中（空结果），不调用检索。
- 括号内标签取值固定：`engine off`、`warming`；百分比 0–100，无总数时不带 chunks。

### D. 前端（WP-147）
- 不新增字段/接口。bind 开关读写现有 `GET/PUT /api/settings/agents/{id}` 的 `tools.code_search.enabled`；只显示库里存的值，不按引擎状态置灰/改写。
- 迁移后主 agent 的 code_search 行存在且为 true，前端不再走“缺行=Off”分支（缺行仍按 Off 显示，作兜底）。

## 子包（排在 WP-140 推送后，不打乱在途）
- Tools：src/tool/availability.rs（code_search 去掉 desired/is_searchable 门禁与 readiness 依赖）、src/tools/code_search.rs（C 表四种返回）。测试：bind=true 时 Warming/未开/失败均可见；四种首行过 v2 正则；非就绪零检索调用；session_search 现有测试不变。
- Runtime（src/config）：global_db seed + 迁移补主 agent code_search 默认绑定（只补缺行）。测试：新库有行=true；已有 false 行迁移后仍 false；子代理无行。
- 硬门：rg 证明 engines/、tools/、serve/ 无写 agent_tools/desired 的运行态路径（保持现状）。
- 护栏：MemAvailable≥6GiB、-j2、共用主 target、不 commit。

## 验收
1. 上述测试全绿；tools/engines/config 现有测试全绿；fmt 0。
2. 场景测试：Warming 期间开始的回合，code_search 可见，调用返回预热首行+空结果；Warm 后同一会话下一次调用返回正常结果（无需点 bind）。

## 交付（2026-10-14）
子包均 done，未 commit（等 GM 推送）。

### WP-148 Tools（A+C）
- code_search 可见性只看 bind；非就绪三种情形 `ToolCallResult::ok` + sync_pct v2 首行 + 空结果零检索。
- 偏差（已批）：失败原因优先引擎真实文本再 fallback 原因码。
- 测试：tools:: 440、availability 11、optional_p0 8、optional_code_search 19、settings_api 48；fmt 0。
- 场景验收 2：`warming_turn_sees_code_search_and_gets_warming_line`（Warming→Warm，bind/desired 全程不动）。
- 既有 ISSUE-055（registry 期望 session_search）仍在，非本包。

### WP-149 Runtime（B）
- 新库 seed 主 agent code_search=true + 标记；老库 `ensure_core_bindings` 一次性只补缺行，成功后写 meta `default_bind_code_search_v1`。
- Q2：新建 primary 且缺键时补 true；显式传值照写；已有 agent 改 primary 不自动绑。
- user_version=7、SEED_REVISION=15 未升。
- 测试：global_db 29、settings_writer 26（含新增 8）、f2_lock_scope 4。

### 前端 WP-147
契约 D 未改接口；请对照 A/C 同步：缺行仍兜底 Off；迁移后主 agent 有行=true；不按引擎状态置灰/改写 bind。
