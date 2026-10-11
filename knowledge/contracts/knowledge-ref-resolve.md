```node
node : knowledge-ref-resolve
status : pending
domain : knowledge
summary : 引用解析：路径→可读→符号/范围→漂移，一引用一状态；错误码固定。
```

**准入：** A1/A2。`resolve` / `validate_with` 按规范顺序取第一个失败状态；状态码表（missing_file、invalid_lines、symbol_drift…）由 cases.json + spec_tests 锁定。解析顺序或状态码漂移会让徽标、提醒与工具 check 对不齐。

依据：[@ file="src/knowledge/validate.rs" symbol="fn resolve"]、[@ file="src/knowledge/validate.rs" symbol="fn validate_with"]、[@ file="src/knowledge/fsref.rs" symbol="fn lookup"]、[@ file="src/knowledge/fsref.rs" symbol="fn read_text"]、[@ file="src/knowledge/spec_tests.rs" symbol="fn resolution_follows_the_spec"]。
上层：[@ key="knowledge"]。
相关：[@ key="knowledge-ref-authority"]、[@ key="knowledge-parse"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- resolve / validate_with —— [@ file="src/knowledge/validate.rs" symbol="fn resolve"]、[@ file="src/knowledge/validate.rs" symbol="fn validate_with"]。
- 状态码锁定 —— [@ file="src/knowledge/spec_tests.rs" symbol="fn resolution_follows_the_spec"]。
