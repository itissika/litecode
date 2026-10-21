# WP-199 实现 — grep / glob / read 精简（硬清冗余）

**分支：** `feat/tool-contract-ptc`  
**口径：** Andy+GM 定稿。不符合本设计的**全部杀掉**，绝对简洁；简洁≠再造方言——用通用易懂语言；**全工具统一 1-based 行号心智**（read 用 `start_line`，不改成 offset）。

## 契约

```ts
tools.grep({ pattern, path?, glob?, literal?, ignore_case?, limit? })
  -> { hits: Array<{ path, line, text }>; message: string }

tools.glob({ pattern, path?, limit? })
  -> { paths: string[]; message: string }

tools.read({ path, start_line?, limit? })
  -> { path, start_line, lines: string[]; message: string }
```

共用不变量：一次有界（limit）；搜到/没搜到；hits=文件+行+一行有界摘要；原因/触顶进 message；**无 cursor / stale_cursor / 多枚举 reason / searched/base/page 树**。

## 验收

1. 三工具按上契约；grep 活跃面无旧方言残留。
2. 行号统一 1-based；教法通用易懂。
3. cargo 绿；artifacts/WP-199；push 实验分支；未合 main。
