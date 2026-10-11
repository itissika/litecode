```node
node : session-write-path
status : pending
domain : harness-core
summary : 会话写入唯一路径：单 writer、operation 幂等优先、revision CAS，回执与变更同事务。
```

**准入：** A3 / A5 / A7。会话库只有一个 writer actor 可写；业务变更、revision 推进与 operation 回执必须在同一事务落地。坏了会出现第二写者、假冲突或「错误回复却已提交」无法凭 operation_id 查回；ADR 0001 与 SQL 死亡清单钉住「禁止旁路开库」。

依据：[@ file="src/session/data/writer.rs" symbol="impl WriterHandle › fn submit_blocking"]、[@ file="src/session/data/writer.rs" symbol="fn execute"]、[@ file="src/session/data/writer.rs" symbol="fn bump_receipt"]、[@ file="src/session/data/sqlite/ops.rs" symbol="fn persist_receipt"]、[@ file="tests/session_data_sql_death_list.rs" symbol="fn sessions_db_sql_and_rusqlite_stay_in_sqlite_module"]。
父节点：[@ key="session-model"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 单 writer —— [@ file="src/session/data/writer.rs" symbol="impl WriterHandle › fn submit_blocking"]；`execute` 在 `BEGIN IMMEDIATE` 内 `dispatch` 后同事务 `bump_receipt`/`persist_receipt`。
- operation 幂等优先于 revision CAS —— [@ file="src/session/data/writer.rs" symbol="fn execute"] 先 `load_operation` 回执，再比 `expected_revision`。
- 删除回执不误伤 revision CAS —— [@ file="src/session/data/sqlite/ops.rs" symbol="fn persist_receipt"]（`CommitKind::Deleted` 确认 sessions 行已消失）；守护：同文件 `fn persist_receipt_deleted_accepts_missing_session_row`。
- 禁止旁路开库 —— [@ file="tests/session_data_sql_death_list.rs" symbol="fn sessions_db_sql_and_rusqlite_stay_in_sqlite_module"]。
