```node
node : context-compact
status : pending
summary : 压缩只改模型看到的历史，不改库里的事实：库里追加一行，旧行原样留着，读的时候才折叠成摘要 + 保留段。
x : 5364
y : 612
domain : harness-core
```

**准入：** A5 / A7。压缩要解决的是「模型看不下」，不是「库里存不下」：库里只追加，不改写任何旧行；模型可见的历史在读取时折叠——压缩点之前换成一段摘要，之后原样保留。落地只有两种动作，都是追加：摘要替换（新增一行 `compacted`）和机械裁剪（超长工具结果、过期媒体各追加一条替换）。除此之外没有第三条改历史的路径。落库后必须从 fold 重载工作集，禁止在内存里拼 summary+kept；提交前失败不写 surface。

0.8 触发边界先在 sealed prefix 上做一次机械裁剪决策（只有单独解除压力才落库，否则不写），再无裁剪地冻结一个 plan：同一模型/指令/工具/前缀/cut/`kept_from_seq`/输出上限。普通压缩与 manual 的输入只含 sealed prefix，manual 准入与 auto 0.8 触发也只看 sealed prefix（完整 working set 仅用于 hard/wait 发送门）；overflow 是同一管线的另一种输入：一开始带完整 surface + tail，摘要输入从最老历史开始丢到新模型物理线的 0.8，最终 checkpoint 仍只落在 sealed 行，tail 留在后面；最终 meter 仍超硬线才是真失败。摘要和机械分支只产出提交计划；公共阶段只提交一次，但提交与交付分开：写库成功是提交事实，读回并校验出可用的 durable surface 才是成功。读回遇到瞬时失败做有限重试；仍读不回来是 unverified，交给 session 再读，不能报成功；读回来但校验坏才停。manual 失败只报错；session 每次取 surface 读失败必须报错停。不回滚 DB；每次结束由 episode 收尾路径发唯一终态。正常 turn 结束或取消都不取消 session episode；只有 revert/delete 取消。硬发送线为 `min(产品窗口, 物理 prompt 上限)`，2% margin 只是提前等待点，不是 no-send 线。

依据：[@ file="src/context_pipeline/compact.rs" symbol="fn compact_input"]、[@ file="src/context_pipeline/compact.rs" symbol="fn commit_plan"]、[@ file="src/context_pipeline/compact.rs" symbol="fn rebuild_committed_surface"]、[@ file="src/session/data/command.rs" symbol="enum SessionMutation"]、[@ file="src/types/death_list_gate.rs" symbol="fn session_seq_g2_pipeline_reloads_fold_not_summary_plus_kept"]、[@ file="src/agent/core.rs" symbol="fn run"]、[@ file="src/session/service/conversation.rs" lines="181-303"]、[@ file="src/client_protocol/controller/session_lifecycle.rs" symbol="impl SessionController › fn start_manual_compact"]。
父节点：[@ key="context-view"]。
相关：[@ key="session-model"]。
