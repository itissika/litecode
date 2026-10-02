```node
node : session-model
status : pending
summary : 会话数据：业务日志为真源；seq 是身份，Item 是载荷，Surface 是派生序。
x : 34
y : 556
w : 416
h : 430
```

**主张：** 会话记录产品事实；身份与顺序长在日志上，不在第二套消息类型里。

- 真源与身份：每行以 `(session_id, seq)` 定位；`seq` 单调分配且不回退——回退删行后，被删的 seq 永不复用。
- Item 是载荷：`item/*` 行的正文是 Responses Item，也是模型视图的输出；提醒、compact、控制面各有自己的业务正文。
- Surface 从日志得到：按 seq 应用追加与替换即可算出（compact 以屏蔽区间表达），是计算不是第二份存储；控制面行不进入模型对话序列。
- 写门唯一：一工作区一个 writer actor 加只读池；业务变更、操作记录与版本推进在同一事务落地，消费者只走类型化命令。
- 幂等先于版本：操作重试（operation_id）取回原回执、不重复执行；版本预期（expected_revision）冲突显式失败，不静默合并。
- 提交边界：错误回复不等于未提交——COMMIT 后收信失败仍可能已落库，只能凭操作身份查回；已封口的正文不可覆写，回退是删行不是覆写。

上层：[@ id="session" label="session"]。相关：[@ id="turn-lifecycle" label="turn 生命周期"]。
依据：[@ file="src/session/data/sqlite/session.rs" label="取号高水位与三原语"]、[@ file="src/session/data/writer.rs" label="写门顺序"]。
