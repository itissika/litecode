# Spec: session seq / surface

LiteCode live 多轮 compact 后乱序、agent 行闪现、历史 FoldCard 被流式误开——根因是 session **第二套身份**。本规格把产品收到：**一条带单调** `seq` **的 append-only 日志是唯一真相**。payload 仍是 OpenAI Responses `Item`。不引入第二套 Message / ContentBlock，不引入 Cordis。

权威细节（立法原文，本机、gitignored）：`dev/plans/session-seq-surface/`。实现与法冲突时改代码。本 `spec.md` 是 tracker 内可版本化的收敛合同；死亡清单 ID 与门禁分层与该目录一致。

## 三层（只许这三层）

```
SessionHeader     日志外元数据。不进模型，不进 surface。
SessionEvent[]    日志。seq = 追加位置，永不改、永不复用。
Surface           哪些事件当前对模型可见、以何顺序可见。
```

非法第四层：`Transcript = Vec<Item>` 当工作集、`checkpoint_seq`+`kept_from_seq` 当模型窗口、`buffer_index` 平行下标、`live-*` overlay、`orderProjection` 双空间排序。

## 身份


| 问题            | 答案                                    |
| ------------- | ------------------------------------- |
| 这一行是谁？        | `seq`                                 |
| 转写怎么排？        | seq 升序（只读 append-origin）              |
| 模型怎么排？        | `surface.nodes`                       |
| 流式 delta 打到哪？ | 该 Item 在 `output_item.added` 已分配的 seq |
| 封口是什么？        | 同一 seq 的 payload 状态变化，不是另开一行          |
| React key？    | `seq`                                 |


`Item.id` / `call_id` 只做适配器与 tool 配对。

## Compact / live

- 成功压缩 = append 一条 `item/user`（摘要），`surface_op = { replace, start, end }`。日志不删被阴影事件。
- Cut 画在 shadowed 边界，不是 replace 事件自己的 seq 尾巴，不是 SQL `ORDER BY compact_checkpoint`。
- 最小对齐：chunk 可只在进程内；**Item 的 seq 在 added 时已分配且不变**。禁止前端用 `item_id` 另开一行。



## 阶段完成 = 架构针绿

`cargo check` / `tsc` **不是** 完成条件，直到最后一张工单。每张工单先把门禁打红再改生产代码。禁止假进度：双写 `seq`+`buffer_index`、给 overlay 加 compact 特例、假 `bufferIndex = committedEnd+i`、SQL 重排后再让前端排回去。

## 明确不做

- 旧 `sessions.db` 迁移（砸库）
- 把 chunk 持久化当身份前提
- 在线协议未改成 seq 之前用前端补丁「修」compact 乱序



## 工单策略

后端先把 log / surface / 协议 / 边路收到针上；前端后做。01–05 不依赖面板能聊。

持久化（原 02）拆成 02a schema → 02b append 读写 → 02c compact+fold 加载。**02c 不可再拆：** 停窗口指针与改 turn 加载必须同一提交，禁止 replace 与 `checkpoint_seq` 双写。