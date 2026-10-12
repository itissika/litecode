# WP-068 / WP-063-runtime（Runtime：ISSUE-036 emit · 类型化压缩失败到 TurnCompleted）

- 父包：WP-063 → WP-064；本包：**WP-068**；owner：runtime；approver：backend_mid
- 状态：**落地完成 → deliver（review）**
- 分支：`ops/team-framework`；**未 commit**
- 产物路径：`.litecode/ops/artifacts/WP-063-runtime.md`（本文件）
- 关联：ISSUE-036（**仍 open**——emit 本包完成，consume 归 Session **WP-069**）
- 禁区遵守：未碰 ISSUE-004、引擎产品；`turn_error` 函数体零改；未发明 wire 字段；未 commit

---

## 1. Goal / Promise（回顾）

修 **ISSUE-036 emit**：auto compact 失败结束回合时，`InternalEvent::TurnCompleted` 携带类型化内存错误类别，使 Session `turn_error` 可判定 `ErrorCode::CompactionFailed`，不再依赖 `final_text` 文案全等。

本包只做 **emit**；**consume** 归 WP-069。

---

## 2. 落地证据

### 2.1 类型与字段

| 项 | 位置 | 说明 |
|---|---|---|
| `TurnErrorKind::CompactionFailed` | `src/runtime/observer.rs` | `#[serde(rename_all = "snake_case")]` → `"compaction_failed"` |
| `TurnCompleted.error_kind: Option<TurnErrorKind>` | 同文件 | `#[serde(default)]`；**不**写入 `turn/end` SessionEvent |
| 不 enrich `TurnEndReason::Error` | — | `reason` 形状未改；落盘仍为 `"error"` 字符串 |

### 2.2 Emit 路径

| 步骤 | 位置 |
|---|---|
| `fn turn_error_kind(&LitecodeError) -> Option<TurnErrorKind>` | `src/runtime/mod.rs`（模块级私有） |
| `finalize_agent_outcome` Error 分支映射后传入 | 同文件 |
| `emit_turn_completed(..., error_kind)` 构造事件 | 同文件；非 Error outcome 恒 `None` |
| 落盘 `EventType::TurnEnd` | **仍只** `turn` / `reason` / `final_text`（无 `error_kind`） |

### 2.3 §3.4-A 机械编译兼容（非 consume）

| 文件 | 改动 |
|---|---|
| `src/client_protocol/project.rs` | match 补 `error_kind: _`（**1 行**；`turn_error` 体未改） |
| `src/client_protocol/controller/mod.rs` | 解构/重构传透 `error_kind`；测构造 `error_kind: None` |
| `tests/client_protocol_project.rs` | 4 处构造补 `error_kind: None` |
| `tests/wire_seq_identity.rs` | 1 处构造补 `error_kind: None` |

`session/manager.rs` / `stage_a_ctx_consistency.rs` 已用 `..` → 免改。

### 2.4 回归测（绿）

```text
cargo test --lib observer::tests
  turn_end_reason_log_schema_is_snake_case_for_every_variant
  turn_error_kind_serializes_snake_case                    # T4
  turn_completed_error_kind_defaults_when_absent           # T3
  turn_completed_roundtrips_error_kind

cargo test --lib turn_error_kind
  turn_error_kind_maps_compaction_failed                   # T1 mapping
  turn_error_kind_none_for_other_errors                    # T2 mapping

cargo test --lib wp025_turn_error
  auto_compact_failure_maps_to_compaction_failed_not_retryable  # 文案路径仍绿（WP-069 再改）

cargo test --test client_protocol_project --test wire_seq_identity turn_
  9 + 1 passed（机械补全后可编可跑）

cargo test --lib controller::
  13 passed
```

跑于 2026-10-12 HKT；定点 `--lib` / 具名 `--test`，未整库 `cargo test`（避免他域未完成编译挡路）。

---

## 3. 明确未做（WP-069）

- 未改 `project.rs` `turn_error` 文案全等逻辑
- 未改 `wp025_turn_error_tests` 断言方式
- 未把 `error_kind` 塞进 JSON-RPC / `agent/turn_finished` params

---

## 4. 交接 Session WP-069（契约锁定）

1. 读 `InternalEvent::TurnCompleted.error_kind`。
2. `turn_error`：`reason == Error && error_kind == Some(CompactionFailed)` → `ErrorCode::CompactionFailed` / `retryable: false`；message 可用 `final_text`。
3. 删除（或降级兜底）`final_text == LitecodeError::CompactionFailed.to_string()`。
4. 更新 `wp025_turn_error_tests`：用 `error_kind` 构造。
5. controller 投影已传透 `error_kind`（本包机械），消费侧按需使用。

交叉审：字段名 `TurnErrorKind` / `error_kind` 与本产物一致；wire 仍只暴露 `StructuredError.code`。

---

## 5. done_when 勾选

- [x] `TurnErrorKind::CompactionFailed` + `TurnCompleted.error_kind`（`serde(default)`）已落地
- [x] `finalize_agent_outcome` Error 路径按 `LitecodeError` 映射类别
- [x] T1–T4 绿；定点命令见 §2.4
- [x] 落盘 `turn/end` **无** `error_kind`；`TurnEndReason` 形状未改
- [x] `turn_error` 函数体本包未改；§3.4-A 仅机械字段补全
- [x] ISSUE-004 / 引擎未碰；**未 commit**
- [x] 本 md 更新为落地证据；ISSUE-036 **保持 open**（emit 历史已记；consume→WP-069）
- [ ] `deliver --id WP-068`（下一步）

---

## 6. 中层批示回顾

- §3.4 **A**（机械补全）✓
- 命名 **`TurnErrorKind` / `error_kind`** ✓
