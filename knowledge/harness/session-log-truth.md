```node
node : session-log-truth
status : pending
summary : 日志为真源：Responses Item 是唯一原子，seq 是门牌，单写者；append/seal/truncate 三原语，视图从日志折叠。
x : 510
y : 1044
```

**会话是一份 append-only 的 Item 日志；库里的是权威。**

- 权威原子：Transcript = Vec<Item>（OpenAI Responses Item），禁止自制 Message / ContentBlock [@ file="src/authority.rs" label="authority.rs"]。
- 身份与顺序：`seq` 是一行的门牌；模块按 seq 键、排序、寻址 [@ file="src/session/event.rs" label="event.rs"]。
- 单写者：一工作区一 writer actor + 只读池；所有消费者走类型化命令，不自己开库 [@ file="docs/adr/0001-session-data-single-writer.md" label="ADR 0001"]。
- 三原语：append（新 seq）/ seal（同 seq 封口）/ truncate（回退：从 user 锚点删除）；compact 是 append with replace [@ file="src/session/data/sqlite/session.rs" label="session.rs"]。
- Surface：按 seq 折叠 append / replace 得到模型可见序——是计算，不是第二存储 [@ file="src/session/surface.rs" label="surface.rs"]。
- 投影可丢：空 assistant、未配对 output 留在日志上；投影可跳过——跳过不是删除。
- 词汇与词义 [@ file="CONTEXT.md" label="CONTEXT.md"]。


