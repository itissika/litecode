# Batch 3 · harness-core（会话/回合）实现对照

负责人：Session。基线：`075308b`。工作区 `/workspace/litecode`，分支 `ops/team-framework`。**未 commit / 未 push。**

## 1. 节点对照表

| key | 结论 | 证据（文件 + 符号 / 测试） |
|---|---|---|
| `session-write-path`（pending） | **一致，无实质漂移** | `WriterHandle::submit_blocking` → 单 writer；`execute` operation 幂等先于 revision CAS；`bump_receipt`/`persist_receipt` 同事务；Deleted 回执确认 sessions 行消失；`sessions_db_sql_and_rusqlite_stay_in_sqlite_module` |
| `session-log-envelope`（pending） | **一致，无实质漂移** | `struct SessionEvent`（seq/type/surface_op/source_seqs/state）；`fold_surface` / `SurfaceOp`；G1 `session_seq_g1_envelope_vocab_matches_mental_model`；Batch3 改 `DATA.md` 消文档噪声（ISSUE-003 closed） |
| `turn-reservation`（pending） | **一致，无实质漂移** | `SessionActivity` Idle→StartingTurn→RunningTurn；`reserve_turn` / `release_turn_reservation` / `start_turn` / `finish_turn`（核 turn_id）；`TurnGuard::begin_turn` 成对；与 agent-loop **分层**不复述 |
| `session-model`（enabled） | **一致**（未改主张） | 真源/seq/Surface/写门/幂等仍由子节点展开；补 `相关：client-projection` |
| `turn-lifecycle`（enabled） | **一致**（未改主张） | 已挂 `turn-reservation`；保留并列 agent-loop 等（GM/Andy 口径） |
| `session`（framework） | **一致**（未改主张） | 契约展开仍指向 session-model / turn-lifecycle |

无结论 B；**无阻断**（blocks=false）。

## 2. 正式 issue

| id | grade/layer | status | 处置 |
|---|---|---|---|
| **ISSUE-003** | drift / product | **closed** | Andy 口径：SessionLog vs SessionEvent = 文档噪声。已改 `src/session/DATA.md` 信封为 `SessionEvent`（含 `type`/`surface_op`/`source_seqs`/`state`），删除「没有平行的 surface_op」。**未改** G1 门禁（辩证：跟代码，不松门）。 |
| （无新开 arch/bug） | — | — | writer 删除回执路径与节点主张一致；未发现新 arch。 |

机读：`.litecode/ops/issues/ISSUE-003.json`。

## 3. 待删 / 待改非知识 md

| 路径 | 建议 | 说明 |
|---|---|---|
| `src/session/DATA.md` | **已改**（本批） | 不再待删；词汇对齐代码后仍是会话域权威心智文档 |
| （无其它本域撞车 md 待删） | — | `.scratch/session-seq-surface/spec.md` 若仍存在仅作历史参考，删前经 GM |

## 4. 互引补全

- `session-model` ↔ `client-projection`：双方补 `相关：`
- `turn-reservation` 挂在 `turn-lifecycle` 下（Batch2 已完成）；与 Runtime `agent-loop` 并列不抢主张

## 5. 本批代码/文档改动（未提交）

- `src/session/DATA.md` — SessionEvent 信封对齐
- `knowledge/contracts/session/{session-write-path,session-log-envelope,turn-reservation}.md` — 追加 §7.2 核实结论
- `knowledge/contracts/session-model.md` — 仅补 `相关：client-projection`
- `.litecode/ops/issues/ISSUE-003.json` — closed

## 6. 阻断声明

**无。** 无 arch；无 core 节点结论 B；未触达 B1–B5。

## 7. kb_check

对本三 pending 节点：`errors=0 warnings=0`（见交付时全库跑次）。
