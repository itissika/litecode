# WP-212 run_code 收尾瑕疵 — brief

分支：`feat/tool-contract-ptc`（不合 main）。来源：Andy 本机实测（WP-211 之后）。**最后一包小瑕疵收尾；范围已钉死。** 不得触碰 `desktop/build/icon.ico`。补测包仍后置。

## 已定范围

**1. P1 `tools.kill` 假成功**

- 仅在确认进程已死后返回 `ok:true`。
- 等退出超时（或进程仍活着）时返回 **`ok:false`**，禁止再报假成功。
- Windows 走 `taskkill /T`。
- `wait` 明确区分「被 kill」与普通 `exit:1`（结构化字段或可解析 message，二选一写进 plan/acceptance）。

**2. P2 未处理 Promise rejection 被静默吞掉**

- 文档曾写 uncaught rejection ends the program；现状既不结束也无提示。
- **最低交付**：程序 settle 时对未处理 rejection 打一条 **`[warn]`**（勿再静默）。
- 是否升级为直接结束程序：plan 里用 Q 问 GM；Andy 已确认 **先 warn** 即可，不必本包做 abort。

**3. 语法**

- `[warn] 1 host calls were still running` → 单数 **`1 host call was still running`**；复数再加 s。

**4. 文档**

- 写清：未知参数（unknown parameter）**整调用失败**，不是「多余字段静默丢弃」。
- 可选一句：沙箱暂无定时器（`setTimeout` / `setInterval`），**本包不实现定时器**。

**5. 可选低危**

- 栈里多一帧越界的 `<eval>`：若改动便宜则压掉；否则跳过，acceptance 注明。

## 验收

1. `cargo test -j 2` 绿（跑前空闲内存 ≥2GiB）；可附 `cargo-full.txt`。
2. 仅为以上项补精简 protected；不堆测试量。
3. PRELUDE / 工具文档与实现一致；SDK 快照若受影响则同步。

## 非目标

- **不实现** 定时器（`setTimeout` / `setInterval`）。
- 不加 `timeout_ms` / `background`、不扩大 run 形态。
- read 分页字段、console.log 加宽、跨调用状态、rm/mv、新工具、合 main。
