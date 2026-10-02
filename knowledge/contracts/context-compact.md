```node
node : context-compact
status : pending
summary : compact：摘要替换已持久 Surface 前缀；历史保留；失败语义以提交边界为准。
x : 990
y : 522
w : 398
h : 720
```

**compact 改变模型历史的有效表达，不伪装成删除历史，也不靠静默少送来过预算。**

### 数据
- `CompactedBody { summary, from, to }` 表达 Surface 半开区间 `[from, to)`；不是日志物理删除区间。
- `SessionMutation::Compact` 带 kept_from 和前缀检查，提交追加新摘要 seq。

### 流转
自动预算触发／手动入口 → 选择已持久前缀与近窗切点 → 摘要调用 → 复检预算 → Compact 提交 → 从真源重载窗口 → 接回未提交尾部。

### 不变量
- 自动 compact 在 step 的预算检查中触发；手动 compact 是独立入口。“显式边界”指走 Compact 契约，不是只许用户点按钮。
- 只摘要已持久前缀；近窗原文不进入被替换的摘要输入，未提交尾部也不能被吞掉。
- 追加摘要并在 Surface 上替换对应前缀，旧日志行仍在；摘要作为 assistant 表达进入 AgentView，不冒充用户发言。
- 摘要调用失败、提交前取消、压缩后仍超硬预算，都不能靠半成品继续请求；恢复局部快照并返回失败。
- 提交服从 revision 与当前模型窗口的前缀长度检查；长度变化必须中止，替换端点必须仍有效。长度检查不是摘要输入的内容哈希校验。
- **COMMIT 是不可逆边界**：提交前失败不产生摘要检查点；提交后的重载失败虽返回错误，已落库的检查点不因此回滚。整个失败路径也可能留下独立的收尾／控制日志。

依据：[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_if_needed" label="自动入口"]、[@ file="src/context_pipeline/compact.rs" symbol="impl CompactPolicy › fn compact_transcript" label="摘要与提交边界"]、[@ file="src/session/model.rs" symbol="struct CompactedBody" label="替换区间"]。
验证：[@ file="tests/working_set_identity.rs" symbol="fn keep_recent_compact_reader_matches_fold_checkpoint_then_kept" label="窗口与折叠一致"]、[@ file="tests/stage_a_ctx_consistency.rs" symbol="fn compact_eats_only_persisted_prefix_and_keeps_uncommitted_tail" label="尾部不被吞掉"]。

上层：[@ id="context" label="context"]。
