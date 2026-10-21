# WP-196 方案 — bash 家族最低风险切口进 Code

**owner：** backend_mid  
**分支：** `feat/tool-contract-ptc`（不合 main）  
**基线：** HEAD 以开工时为准（现 `242c5c9`）  
**依据：** brief；WP-195 dig §7 门槛；Andy 纠正（最终要进）+ **2026-10-21 Andy 再改口径**（Direct bash 家族默认关、不做双份、不教两套）

---

## 0. 口径补丁（2026-10-21，覆盖原 Q2/§3）

**立即生效（Andy / GM）：**

1. Direct `bash` / `wait_shell` / `kill_shell`：**默认关掉**（默认投影/绑定不暴露给 agent，或等价默认 off）。
2. **不做双份、不教两套**：Code 侧只推 `run_command`；禁止「短测走 Code、长作业走 Direct」双轨文案。
3. 三件套实现代码若保留，须 **默认 off 且对 agent 不可见**（极少开关/逃生）；非本包教法内容。
4. 原方案「三件套暂留长作业 + 教法收窄」**作废**；Runtime/Tools 已写双轨部分重做。

阻塞（必须保留某种长作业）先报 GM；**默认按 Andy：关**。

---

## 1. 切口选择（最低风险）

**选调研 B：新增 Code 面「前台可取消结构化执行」工具 `run_command`；Direct 三件套默认对 agent 关闭。**

| 候选 | 判定 |
|---|---|
| A. 只改 `surface()` | **禁止** |
| B. 新前台结构化工具 + Direct 默认关 | **本包采用** |
| C. 三件套整体 code 化 | **不做**（非最低风险） |

**工具名：** `run_command`（Code only）。SDK：`tools.run_command(...)`。

---

## 2. 目标契约（对照 §7 本切口子集）

1. **成功 `data`：** `status` / `exit_code` / 有界 stdout·stderr（或 `output_file`）；非零 exit 仍 resolve ok；取消/超时走 error。
2. **等待不堵预算：** async 等待；取消→杀进程组；超时杀进程组（不转后台）。
3. **程序结束：** 不登记后台；无跨调用 `bash_id`。
4. **effects：** 会改文件则按惯例声明。
5. **长日志：** 摘要 + `output_file`，用 `tools.read` 分页。
6. **同面 / 唯一命令路径：** agent 默认只见 Code `run_command`，不见 Direct bash 家族。

**cwd：** `workspace_root`。

---

## 3. 防退化（产品硬要求）

- 「改文件 → 跑命令 → 看退出码」在同一 `run_code` 内用 `edit/write` + `run_command` 完成。
- **默认投影无 Direct bash 三件套**，从根上堵住「凡事 Direct bash」逃逸。
- 教学/SDK **只**描述 `run_command` 命令路径。
- acceptance：agent 视角场景 + 证明默认不可见 Direct bash（测或投影断言）。

---

## 4. 分工

| 域 | 职责 |
|---|---|
| **Runtime** | `run_command` 实现（已有）+ **默认隐藏 Direct bash 家族** + explore/general 绑定 + 投影/准入测 + sdk_snapshot |
| **Tools** | **重做**教法：只推 `run_command`；废除双轨 bash 文案；fixture 对齐 |
| **Provider** | 默认不派 |
| **mid** | 串槽、交叉审、acceptance、commit/push、deliver |

---

## 5. 验收

1. Code 可调 `run_command`；稳定 `data`；预算/取消/超时测绿。
2. **默认 agent 不可见** Direct `bash`/`wait_shell`/`kill_shell`。
3. 教学/SDK 单轨；防退化场景说明。
4. `cargo test` 相关绿；artifacts 齐；push 实验分支；未合 main。

---

## 6. 拍板（更新）

| # | 议题 | 默认 |
|---|---|---|
| Q1 | 名 `run_command` | 采用 |
| Q2 | Direct 三件套 | **默认关 / agent 不可见**（实现可留） |
| Q3 | 不做三件套整体 code 化 | 是 |
| Q4 | timeout_sec 1–120 默认 30 | 已定 |
| Q5 | effects 最小声明 | 是 |
| Q6 | 双轨教法 | **禁止** |
