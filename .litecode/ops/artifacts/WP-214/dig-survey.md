# WP-214 dig-survey（fill 前只读草图；结论以批准后 report 为准）

基线产品 tip 参考：`2735592`。下列为路径索引，不作断裂判定。

## 触发

- 手动 UI：`web/src/components/ContextUsageRing.tsx`（compact 按钮 / `compactEligible`）→ `web/src/stores/turnStore.ts`（`compacting`、`compact_session`）
- 手动 RPC：`src/client_protocol/controller/session_lifecycle.rs` `start_manual_compact`；`connection.rs` ~1004
- 自动：`src/context_pipeline/context_meter.rs` `should_compact`（`used_tokens > compact_at`）→ `compact.rs` `compact_if_needed` / `compact_now` / `compact_transcript`

## 持久 / 指针

- `src/session/data/sqlite/session.rs`
  - `write_compact_pointers` ~861
  - `refresh_compact_pointers_from_log` ~886（`kind = 'compacted'`）
  - `apply_compact_checkpoint_checked` ~3376（admit kind `"compacted"`，`CompactedBody`；draft `surface_op: None`）
- `src/session/model.rs` `CompactedBody`
- `src/session/working.rs` CompactionCheckpoint ↔ 模型可见投影
- `src/session/DATA.md` compacted 行语义（切痕 / 摘要展示约定）

## 出站

- `src/client_protocol/project.rs`（surface_op 投影）
- `src/client_protocol/protocol.rs` `CompactSession` / compaction 事件码
- FE 类型：`web/src/api/types.ts` `kind: "compacted"`、`compact_session`

## FE 消费

- `web/src/api/adapter.ts` `isCompactCutRow` / `transcriptMarkKind` → `"compact_cut"`
- `web/src/components/transcriptMarks.tsx` `CompactCutMark`（summary 非空才可点）
- `web/src/components/MessageList.tsx` `TranscriptMarkForRow` + 瞬时 `CompactingMark`
- 相关测：`MessageList.test.tsx`、`transcriptMarks.test.tsx`、`messageStore.test.ts`、`turnStore.test.ts`

## 契约

- `docs/compaction-architecture.md`（单执行路径；prune 可无 LLM；成功 checkpoint = role-free `compacted` 行）
