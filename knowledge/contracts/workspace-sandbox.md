```node
node : workspace-sandbox
status : pending
summary : 工作区路径唯一安全门：相对路径经 Sandbox::resolve，越界即 Escape，不可放宽。
x : 5094
y : 2178
domain : ide-substrate
```

**准入：** A6。工作区沙箱是全局唯一路径边界——人类 API、agent SAFE 与 LSP 路径共用同一解析原语，只有策略入口不同；绕过 `Sandbox::resolve` 等于绕过整仓工作区门。坏了会让文件操作、引擎与工具对“何为工作区内”各说各话，并直接触及 B3 安全阈值。

依据：[@ file="src/workspace/sandbox.rs" symbol="impl Sandbox › fn resolve"]、[@ file="src/workspace/sandbox.rs" symbol="mod tests › fn rejects_parent_dir_escape"]、[@ file="src/workspace/tool_path.rs" symbol="fn resolve_human_relative"]。
上层：[@ key="workspace-shared-substrate"]。
基线：075308b

**核实结论：一致，无漂移。**（基线 075308b）
- `Sandbox::resolve` 经 `resolve_human_relative`，越界为 Escape —— 守护 [@ file="src/workspace/sandbox.rs" symbol="mod tests › fn rejects_parent_dir_escape"]（通过）。
- 人类 / Agent SAFE / LSP 共用同一解析入口 —— [@ file="src/workspace/tool_path.rs" symbol="fn resolve_human_relative"]。
- ISSUE-010：`copy_path` 在 resolve **之后**丢了「源是 symlink」信息；边界本身未放宽、未第二实现。修复应在 service 层先 `symlink_metadata`，或提供不跟随的 resolve 变体后再拒链。
