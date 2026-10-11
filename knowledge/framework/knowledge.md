```node
node : knowledge
status : enabled
summary : 知识框架：关系可验证；磁盘为引用权威；后端索引为唯一真相。
x : 1728
y : 504
w : 480
h : 398
domain : knowledge
```

**准入：** A3。知识库的价值在稳定、可验证的关系（节点↔节点、正文↔文件/符号），而不是散文；引用是否成立只看磁盘物理状态，结论只认后端索引，前端只消费快照。坏了则 turn 提醒、knowledge 工具与 API 会对同一仓库给出不同答案。

依据：[@ file="docs/knowledge-references.md"]、[@ file="src/knowledge/mod.rs"]。
上层：[@ key="product-brief"]。
相关：[@ key="featured-tools"]。
契约展开：[@ key="knowledge-ref-authority"]、[@ key="knowledge-parse"]、[@ key="knowledge-index"]、[@ key="knowledge-ref-resolve"]、[@ key="knowledge-tool"]。
基线：075308b
**核实结论：一致，无漂移。**（基线 075308b）
- 框架承诺：磁盘权威 + 后端索引唯一真相 —— 规范 [@ file="docs/knowledge-references.md"]；实现根 [@ file="src/knowledge/mod.rs"]。
- 契约展开五子节点均核实为一致；前端投影见他域 [@ key="knowledge-client-view"]。
