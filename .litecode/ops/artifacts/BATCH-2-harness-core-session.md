# Batch 2 · harness-core（会话/回合）目录清单

负责人：Session。基线核对：`075308b`（分支 `ops/team-framework`）。  
范围：`src/session/**` 的会话真源 / 写入 / 预占；**不含** context_pipeline/reminder（上下文）、runtime/agent 主环（Runtime）、工具子契约（Tools）。

## 1. 拟新增节点

| key | 路径 | 上层/父节点 | A# | 一句话承诺 |
|---|---|---|---|---|
| `session-write-path` | `knowledge/contracts/session/session-write-path.md` | 父节点 `session-model` | A3/A5/A7 | 单 writer；operation 幂等优先于 revision CAS；回执与变更同事务；禁止旁路开库 |
| `session-log-envelope` | `knowledge/contracts/session/session-log-envelope.md` | 父节点 `session-model` | A1/A7 | 日志行信封是 `SessionEvent`（seq/surface_op/state）；Surface 由 fold 派生；G1 词汇门 |
| `turn-reservation` | `knowledge/contracts/session/turn-reservation.md` | 父节点 `turn-lifecycle` | A2 | Idle→StartingTurn→RunningTurn；spawn 失败释放；finish 核 turn_id；与 TurnGuard 成对 |

均已落盘，`status: pending`，`domain: harness-core`，`基线：075308b`。`kb_check --node` 对本三节点 errors=0 warnings=0。

## 2. 拟保留 / 拟改存量

| 节点 | 处置 |
|---|---|
| `session`（framework） | **保留**主张。补 `domain: harness-core`、`基线：075308b`。契约展开仍指向 session-model / turn-lifecycle。 |
| `session-model` | **保留**主张（真源/seq/Surface/写门/幂等/提交边界）。补 domain、基线；补契约展开挂两个子节点。 |
| `turn-lifecycle` | **保留**主张。补 domain、基线；在既有契约展开（Runtime 已挂 agent-loop 等）前列入 `turn-reservation`。 |
| `subagent-lifecycle` | **保留**，本轮不拆。所有权边界已由 ADR 0003 钉住；与 Tools 交叉处本轮只引用、不新建。 |

## 3. 疑似主节点重复

- 本域新增三节点与 `session-model` / `turn-lifecycle`：**主张分层**（写路径 / 信封 / 预占状态机），未与父节点复述同一不变量为第二主节点；`kb_check` 未报对本三节点的 `dup_primary_suspect`。
- `turn-lifecycle` 同时展开 Runtime 的 `agent-loop` / `stream-projection` / `turn-llm-binding`：属跨域挂树，主节点仍归 Runtime；Session 只挂预占子节点，不抢主环主张。
- 存量 tools 域大量 `dup_primary_suspect` 与本域无关，不纳入本清单。

## 4. Issue 草稿

| 关联 | 说明 |
|---|---|
| **ISSUE-003**（已种子，`drift` / product / harness-core / core=session-model） | `src/session/DATA.md` 仍用 SessionLog 词汇，代码真源是 `SessionEvent`；G1 `session_seq_g1_envelope_vocab_matches_mental_model` 失败。本轮节点 `session-log-envelope` 把信封主张钉在代码侧；Batch 3 定 DATA.md 改词或放宽门——**不在本轮改 DATA.md / death_list**。 |
| （无新开） | 本轮扫码未发现新的 arch/bug；writer 删除回执与 revision CAS 近期已修，与 `session-write-path` 主张一致，Batch 3 再核测试守护是否够。 |

## 5. 刻意不建

- Live 专节点：并入 `turn-reservation`（活动态 ≠ 日志真源）。
- seam / reminder：归上下文域。
- subagent hub 细节：ADR 0003 + 既有 `subagent-lifecycle` 足够。
