# 03: Loop / prepare 只读 surface

**What to build:** 每一步打给模型的内容，只能从当前 session 日志 fold 出来：`derive_messages(fold_surface(log))`。内存里不再有一份无 seq 的 `Vec<Item>` 当真相，也不再把 compact 结果写成 `[summary]+kept` 再喂给下一步。前端和线协议仍可暂时残破。

**Blocked by:** 02: 持久化权威是事件日志

**Status:** ready-for-agent

- [ ] `begin_turn` / prepare 从事件序列 fold，不从 SQL 重排结果或 checkpoint 指针取窗口
- [ ] compact 成功后工作集不是改写后的 `transcript = [summary]+kept`（B3）
- [ ] `HotView` / pipeline 若仍暴露 Item 切片，必须是派生且命名表明是 `model_items`，禁止当 persist/FE 数据源（B4、B7）
- [ ] 游标是 seq（`committed_next_seq` / `persisted_max_seq`），不是 `items.len()`（B5）
- [ ] persist 插入的是 `SessionEvent`，seq 来自分配器
- [ ] G2：context pipeline 与 agent 模型加载路径上 B1–B6 清零；P2 探针绿
