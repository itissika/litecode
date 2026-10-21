# WP-197 实现 — shell 三动作精简落地（硬清历史）

**分支：** `feat/tool-contract-ptc`（实验分支；未要求合 main）  
**口径（Andy + GM 聊天定稿）：** 按新契约落地；**不符合本设计的实现/描述/SDK/测试/绑定，全删，一行不留**（硬要求，不清历史尾巴）。

## 契约（唯一）

三个动作：`run` / `wait` / `kill`（最终工具名可微调，但语义不变）。

```ts
tools.run(command: string) -> Command
tools.wait(id: string, sec: number) -> Command  // sec 必填；超内置最大值静默钳制
tools.kill(id: string) -> Command

type Command = {
  id: string;
  exit_code: number | null;  // null=未结束/无码；数字=已退出
  output: string;            // 主感知区：始终有；截断/落盘路径/被杀说明都写进文本
};
```

- 无 kind 三态、无 timeout/promote、无 run_in_background、无 stdout/stderr 双窗、无 command_output 方言。
- 删除对象包括但不限于：现行 `run_command` / `command_output` / `command_kill` 及一切旧 bash/wait_shell/kill_shell 残留、相关 description、SDK fixture、seed 绑定、权限 preset 名若只服务旧契约。
- wait：有界；模型给的 sec 超最大值 → 静默钳到内置上限（上限写入说明，对模型可见）。

## 验收

1. Code 面仅上述三动作；默认绑定只有它们（shell 相关）。
2. 仓库内无旧 shell 契约符号/文案残留（grep 验收）。
3. 相关 cargo test 绿；artifacts/WP-197/{acceptance,diff-summary}.md。
4. push 实验分支；未擅合 main。

## 非范围

- 交互终端、跨会话长作业产品另立；本包不做半等以外的扩展。
