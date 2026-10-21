# WP-198 实现 — edit / apply_edits 精简契约（硬清）

**分支：** `feat/tool-contract-ptc`  
**口径：** Andy+GM 定稿；**不符合本设计的必须全杀，绝不错过**。现行实现里对模型有用的状态反馈/指引（失败原因文案等）该留的留，塞进 `message`，不要堆结构化 details 树。

## 契约

```ts
tools.edit({ path, old_string, new_string }) -> EditResult
tools.apply_edits(edits: Array<{ path, old_string, new_string }>) -> { results: EditResult[] }
// 串行；单项失败跳过继续（继承现行部分失败语义）

type EditResult = { ok: boolean; path: string; message: string };
```

砍：`replace_all`、首期 `expected_hash`、blocks/applied/failed/details、半成功 resolve vs 全失败 throw 双出口。旧单工具多块 API 删除，一行不留。

## 验收

1. Code 面仅 edit + apply_edits；grep 无旧契约残留。
2. 失败跳过续跑；message 保留有用指引。
3. cargo 绿；artifacts/WP-198/{acceptance,diff-summary}.md；push 实验分支；未合 main。
