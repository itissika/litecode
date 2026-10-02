```node
node : session-write
status : pending
summary : 会话写门：单写者、revision 冲突检查、operation_id 幂等；提交与收信是两件事。
x : 272
y : 530
w : 398
h : 720
```

**业务变更、操作记录和 revision 在同一个会话数据库事务里落地。**

### 数据
- `SessionMutation` 是封闭写命令；`expected_revision` 是会话预期版本，`operation_id` 是同一次操作的重试身份。
- `CommitReceipt` 描述已接受操作；revision 按接受的新命令推进，不保证正文一定变化。重放已有 operation 不再推进。
- `SessionDataReader` 是不能升级为 writer 的只读面。

### 流转
命令入队 → 查询已有操作 → 检查 revision → BEGIN → 执行业务及记录回执／change log → COMMIT → 回复 → 消费者更新投影和通知。

### 不变量
- 一个工作区由单 writer actor 串行写；消费者走类型化命令，不自己建立业务写路径。
- 幂等识别先于 revision 检查：同一次操作的重试能取回记录，不能因旧 expected_revision 再执行一遍。
- revision 不匹配显式冲突，不静默合并；回退导致工作集尾部作废也必须服从写门。
- 只有 `in_progress` 行可更新、封口；已 settled 的正文在运行期不可重写，新内容需新行。truncate／删除是移除，不是覆写。
- 提交前失败回滚事务、丢弃超前的 writer 缓存；文件清理在 COMMIT 后进行。
- **错误回复不等于未提交**：COMMIT 后收信失败仍可能已落库。只能用原 operation_id 查回／重试，不凭错误重复副作用；回执的临时投影字段不是幂等身份。

依据：[@ file="src/session/data/command.rs" symbol="enum SessionMutation" label="写命令"]、[@ file="src/session/data/writer.rs" symbol="fn execute" label="事务与幂等"]、[@ file="src/session/data/sqlite/session.rs" symbol="fn assert_stream_row_rewritable" label="settled 守卫"]。
验证：[@ file="tests/session_data_fault.rs" symbol="fn fault_after_commit_keeps_rows_and_operation_is_idempotent" label="提交后回复失败"]、[@ file="src/session/data/final_immutability_tests.rs" symbol="fn a_finalised_row_is_never_rewritten" label="封口不可覆写"]。

上层：[@ id="session" label="session"]。
