# WP-197 方案 — shell run/wait/kill 精简契约（硬清历史）

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** HEAD `ab95a80`（已删 Direct bash；现存 `run_command` / `command_output` / `command_kill` 为本包删除对象）  
**依据：** brief；Andy+GM 定稿硬清

---

## 1. 目标契约（唯一）

```ts
tools.run(command: string) -> Command
tools.wait(id: string, sec: number) -> Command  // sec 必填；超上限静默钳制
tools.kill(id: string) -> Command

type Command = {
  id: string;
  exit_code: number | null;  // null=未结束/无码
  output: string;            // 始终有；截断/落盘路径/被杀说明写入文本
};
```

- Code 面仅上述三动作；默认 shell 相关绑定只有它们。
- 无 kind 三态、无 timeout/promote、无 run_in_background、无 stdout/stderr 双窗、无 command_output 方言。
- **wait 上限默认 300s**（写入 description；模型给更大值静默钳到 300）。

**工具注册名：** `run` / `wait` / `kill`（SDK `tools.run` / `tools.wait` / `tools.kill`）。若与既有非 shell 符号冲突，中层拍板加前缀 `shell_` 再报 GM——默认不加。

---

## 2. 硬清范围（一行不留）

删除/改写直至 grep 干净：

| 删 | 含 |
|---|---|
| 实现 | `run_command`、`command_output`、`command_kill`、相关 `command_job` 若只服务旧契约 |
| 文案 | 对应 descriptions、Deprecated bash/wait_shell/kill_shell 残留 |
| 绑定 | seed / registry / permission preset 名若只服务旧契约 |
| 测/SDK | fixture、sdk_snapshot、相关 cargo 测改到新契约 |

可复用底层：进程组、job 登记、输出截断/落盘——**仅作内部实现**，对外只暴露 Command 三字段。

---

## 3. 行为要点

1. **run**：启命令 → 立即或短等后返回 Command（未结束则 `exit_code=null`，`output` 已有片段或说明）；登记可 wait/kill 的 id。
2. **wait**：必填 sec；钳制后阻塞至结束或到期；到期未结束仍返回 Command（null exit + output 现状）。
3. **kill**：杀进程组；返回 Command（被杀说明进 output）。
4. **output**：主感知区；截断标注、落盘路径、被杀/超时说明全写进字符串；字段永远存在（可空串，不缺字段）。

---

## 4. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | 新三工具实现 + 硬删旧实现/绑定/测；底层 job 复用；投影默认仅三动作 |
| **Tools** | description + SDK fixture + 单轨教法（只教 run/wait/kill）；扫残留文案 |
| **mid** | 串槽、交叉审、grep 验收、acceptance/diff-summary、commit/push、deliver |

串行：Runtime 先落实现与删除 → Tools 对齐教法/fixture（可叠 description 草稿）→ mid 收口测与 artifacts。

---

## 5. 验收（对照 brief）

1. Code 默认绑定仅 `run`/`wait`/`kill`（shell 相关）。
2. 仓内无旧符号/文案：`run_command`、`command_output`、`command_kill`、旧 bash/wait_shell/kill_shell 契约残留（grep）。
3. 相关 cargo test 绿；`artifacts/WP-197/{acceptance,diff-summary}.md`。
4. push `feat/tool-contract-ptc`；未合 main。

---

## 6. 拍板

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 工具名 `run`/`wait`/`kill` | 采用 |
| Q2 | wait 上限 300s 静默钳制 | 采用 |
| Q3 | 硬删现行 run_command 三件套，无双轨/无逃生 | 是 |
| Q4 | 不合 main | 是 |

