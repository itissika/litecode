# Bot 阶段一：内部一等能力

## 目标

让 bot 成为 LiteCode 内部真实可用的一等产品能力：

- 用户能从侧边栏创建 bot；
- bot 有稳定身份和默认 session；
- bot 能发现其他 bot，并给 bot 发消息；
- bot 的会话走现有 `SessionService`，不新增 session 内核；
- bot 工具集只对 bot 会话生效。

阶段一的重点是“好用和可验证”，不是一次性做全 bot 平台。

## 范围

### 要做

- Bot 身份：名称、归属、可见性、能力。
- Bot 默认 session：沿用普通 session 的 turn、事件、权限、取消和历史。
- 侧边栏“新建 bot”入口。
- Bot 目录与最小发现能力。
- Bot 工具集：创建 bot、列出/发现 bot、给 bot 发消息。
- Bot-to-bot 消息复用普通消息、队列和回合。
- Bot 身份的持久化；至少保证重启后仍可识别和继续使用。
- 普通 session 与 bot 会话的工具面隔离。
- 基本的创建配额、可见范围和递归创建约束。

### 不做

- 不做 ACP 适配器。
- 不做飞书适配器。
- 不做远程身份、多租户、OAuth 或公网接入。
- 不做全局 bot 网络、市场或跨工作区发现。
- 不做新的 session 类型、runtime、writer 或消息总线。
- 不把 bot 字段写进 `SessionService` 的会话模型。
- 不让普通 session 获得 bot 工具集。
- 不把 bot 发现做成无边界的社会化网络。

## 关键约束

- Bot 是产品身份，session 是运行对象。
- 一个 bot 先有一个默认 session，但 `bot_id` 与 `session_id` 不写死相等。
- 所有会话动作通过 `SessionService`。
- Bot 工具集是产品层适配，不是新的 session service 方法。
- Bot 发现属于 bot 产品层，不属于 session 列表。
- Bot-to-bot 通信复用现有队列和回合，不另造通信总线。
- Bot 创建 bot 必须有可见性、配额和递归限制。
- 普通 session 完全不受 bot 工具和 bot 目录影响。

## 验收

阶段一完成时，至少能证明：

1. 用户从侧边栏创建 bot，重启后仍能找到它。
2. 一个 bot 能发现另一个可见 bot。
3. 一个 bot 能给另一个 bot 发消息，并收到同一个会话语义下的回合结果。
4. 停止、取消、权限和失败行为与普通 session 一致。
5. 普通 session 看不到、也不能调用 bot 工具集。
6. bot 的创建和消息链路没有绕过 `SessionService`。
7. bot 之间不会因为递归创建或无边界发现形成失控循环。
8. 用户愿意日常使用它，而不是只跑一次演示。

## 退出条件

- Bot 能作为内部产品稳定使用。
- Bot 产品层与 session 核心边界清楚。
- 出现的问题主要在 bot 产品策略，而不是会话语义或写入路径。
- 下一阶段可以直接在不动 bot 内核的前提下接 ACP 或飞书。
