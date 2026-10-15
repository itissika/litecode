# WP-183 实现方案（backend_mid）— 落地 WP-180 Obs-1 / Obs-2

**依据：** WP-180 done（Obs-1/Obs-2 建议拆包）；Andy/GM 开实现。  
**可引用：** WP-175 / WP-178 H1 锁定（合成 fixture；实网未证）— **本包不修 codec**。  
**产物：** `.litecode/ops/artifacts/WP-183/`

---

## 1. 目标

落地「可崩但留证据」，优先 **观测 / 尽量不改控制流**：

### Obs-1（进程与工具可见性）

1. **Panic hook**：进程级 hook，写 `crash-<unix>.log`（栈 + 若可得的 session/turn）到约定 diagnostics 目录。
2. **Diagnostics 目录约定**：例如 `<data_root>/diagnostics/<session_id>/` — `last-turn.json`（turn_id、phase、last_tool、last_error）、`crash-*.log`、指向 `LITECODE_LLM_WIRE/<session_id>/` 的说明或 symlink。
3. **工具调用审计（observe-only）**：append-only 文件或固定 tracing target；字段含 `phase=start|auth_ask|auth_done|exec_done|panic|cancel`、`session_id, turn_id, step, tool, call_id, rule_id?`、结束 `ok|error` + 短 error 类。目标：回答「grep 进没进过 / 卡在 auth 还是已返回」。

### Obs-2（TurnError 可关联 + runbook）

1. **TurnError / GiveUp ↔ wire**：日志打出 wire 相对路径（若 `LITECODE_LLM_WIRE` 已开）；结构化字段可关联 session/turn。
2. **失败保留策略**：文档化 / 可选「仅失败请求保留」降体积（实现以最小扰动为准；默认可先打路径+保留现有 retention）。
3. **Luna + grep 排障 runbook**：写入 artifacts（开 wire → 复现路径 → 收 wire/审计尾/transcript 尾/crash/进程是否仍在/浏览器 console → 分类 A/B/C）。

---

## 2. 域归属与拆分

| 子包 | owner | 内容 |
|------|-------|------|
| **WP-188**（WP-183-a） | `runtime` | Obs-1 panic hook + 工具审计埋点（runtime/tool pipeline 侧）+ Obs-2 TurnError/GiveUp 关联日志；diagnostics 路径约定落地（读写 data_root 约定） |

**为何单子包、不拆 session：** diagnostics 目录与 wire 互指以约定+runtime 写入为主；session 持久化路径若需只读辅助，由 runtime 经既有 data_root API / 短 handoff，避免与 WP-181 同时改 `src/session` writer。

**串行：** 本包内部 Obs-1 → Obs-2（hook/审计先于关联日志与 runbook 验收）。与 **WP-181 重型互斥**（不同时占编译/测槽）。

---

## 3. 显式非目标

- **不修 H1 codec**（unknown type、补建 `*_tokens_details`、unknown-event 策略）— 除非实网 wire 已确认且 **另立 Fix-H1**。
- **不改权限桥 / AskOutcome / reject_all / waiting_for_user** — 归 **WP-181**。
- 不改 FE ErrorBoundary（若证 C，前端另包）。
- 不把 WP-175 与 WP-176 绑因果。

---

## 4. 实现边界（稳健性）

| 做 | 不做 |
|----|------|
| 加观测、约定目录、审计行、TurnError 关联字段 | 借机大改工具控制流 |
| 确认现有 `catch_unwind` / pipeline join→error 覆盖并补审计字段 | 假装修根因 / 无证据改 codec |
| runbook + 一次手工或测记录 | 强制默认对全体用户开 wire |

---

## 5. 重型与排槽

- `LC_MIN_MEM_GIB=2` `LC_KILL_MEM_GIB=1`；`CARGO_BUILD_JOBS` 按团队口径（常 2；紧张用 1）。
- **与 WP-181 互斥排槽**；向 backend_mid 报槽后再跑。

---

## 6. 验收（done-when）

- [ ] panic / diagnostics 路径约定落地且可演示（至少 hook 或等价写入 + 目录说明）
- [ ] 工具审计能回答「某次 grep 是否 start/auth/exec」
- [ ] TurnError/GiveUp 日志能指到 wire 路径（在 wire 开启时）
- [ ] Luna+grep runbook 写入 `.litecode/ops/artifacts/WP-183/`
- [ ] 相关测或手工验收记录；**未**夹带 H1 codec 大修；**未**改权限桥
- [ ] 与 WP-181 无文件冲突说明（若有重叠，注明只读/未改）

---

## 7. 子包 brief 草稿路径

- `.litecode/ops/artifacts/WP-183/183-a-brief.md`（runtime）

父包 `fill --kind split` 后 submit；子包 drafting，待父包 approve 后由 `runtime` fill→submit（approver=`backend_mid`）。
