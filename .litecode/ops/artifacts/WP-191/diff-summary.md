# WP-191 diff 摘要（main...2090cc0）

## git diff main...HEAD --stat
```
 .litecode/ops/artifacts/WP-191/acceptance.md |  21 +
 .litecode/ops/artifacts/WP-191/impl-notes.md |  55 ++
 .litecode/ops/artifacts/WP-191/impl.diff     | 786 +++++++++++++++++++++++++++
 src/client_protocol/project.rs               |  34 ++
 src/client_protocol/protocol.rs              |  36 ++
 src/llm/mod.rs                               |   4 +-
 src/llm/replay_compat.rs                     | 101 +++-
 src/runtime/exec.rs                          | 358 +++++++++++-
 src/runtime/observer.rs                      |  36 ++
 9 files changed, 1394 insertions(+), 37 deletions(-)
```

## strip/compact 行为确认

- `src/llm/replay_compat.rs::strip_foreign_ciphertext`：返回类型 `usize` → `StripCiphertextStats { ciphertexts_stripped, reasoning_wholly_omitted }`。判定 own/foreign 与 `reasoning.encrypted_content = None` 的动作原样保留；新增分支只在已清密文时累加只读计数（`endpoint_type == "messages" || !reasoning_has_text`），不修改 Item。
- 新增 `reasoning_has_text`（纯读 helper）。`llm/mod.rs` 仅多导出 `StripCiphertextStats`。
- `strip_rebased_signatures`、`retain_own_reasoning`：函数体未改；exec.rs 调用点逻辑（Summary 走 retain_own、否则走 strip_foreign；rebased 先 strip）顺序与分支不变，只是改为读 `stats` 并多传出 `reasoning_wholly_omitted`。
- compact 相关文件不在 diff 中，未改。
- replay_compat 既有测仅改为 `.ciphertexts_stripped` 取值，另增 2 测（messages wholly-omitted / responses 保留文本不计）。11/11 绿。

## 新增（观测/告警）

- `runtime/exec.rs`：`LlmRequestHealthInput` + `classify_llm_request_health`（severity silent/debug/toast），emit 于 `LlmRequestBuilt`；`projection_rewrote` 固定 false；`request_health_tests` 5 测。
- `runtime/observer.rs`、`client_protocol/{protocol,project}.rs`：Wire 字段扩展 + toast 时另发 `SnapshotNotice` warn。
- 分支 commit 内含 artifacts：`acceptance.md`（初稿）、`impl-notes.md`、`impl.diff`。
