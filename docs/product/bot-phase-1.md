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

- 不做外部平台接入。
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
- Bot 创建 bot 必须有工作区边界、配额和递归限制。
- 普通 session 完全不受 bot 工具和 bot 目录影响。

## 阶段一实现约定（已确认）

以下决策在开工前已确认，用于约束实现，不改变上面的边界：

- **能力**：创建时从用户选择的模板 primary agent 复制一个独立的 owned profile（id 前缀 `bot_`），模型、系统提示、工具和步数归该 bot 所有；bot 只额外获得 bot 工具，不新增能力模型。
- **消息**：`bot_send` 把消息排入目标 bot 的默认会话后立即返回，不等待目标回合结果；目标回合在自己的会话里完成，需要回信时由目标 bot 再调用 `bot_send`。
- **可见性**：bot 只在当前工作区可见，不引入 owner/workspace 可见性字段；管理 bot 由用户负责。
- **归属与配额**：用户创建的 bot 深度为 0；bot 创建的 bot 深度为 1 并记录父 bot，不能继续创建。每工作区最多 16 个 bot。
- **持久化**：bot 身份存 `.litecode/bots.json`，与 `sessions.db` 同目录；默认 session 被删除后，下次打开 bot 自动重建；注册表写入失败时删除刚创建的空 session，不留下无归属会话。
- **工具面**：bot 工具为 `bot_create`、`bot_list`、`bot_send`，只追加到 bot 会话，不进入普通能力目录和 Settings；depth-1 bot 不再发布 `bot_create`。
- **侧边栏**：bot 行与 session 行共用条目呈现和文件夹树；bot 的默认会话不再重复出现在普通会话列表。

## 最小闭环实现约定（本轮）

- **独立 profile**：bot 只写自己的 `bot_...` AgentProfile；模板 profile 只读复制，绝不共享。`bot_` 前缀 profile 从普通 primary 选择器和 Settings → Agents 列表过滤，但 `get_agent` 仍可按 id 供 bot 编辑器读取。
- **迁移**：旧 `bots.json` 里的 `agent_id` 视为模板 id；启动时逐个复制为 owned profile，并同步默认 session 的 agent_id。只有该 bot 自己的 `bot_<id>` profile 不重复复制；其他 bot 的 profile 或共享模板都会复制为自己的。
- **更新/删除**：`bot/update` 覆盖身份、文件夹/排序和能力 profile；运行中的 bot 拒绝能力变更。`bot/delete` 删除默认 session、owned profile 和 bot 记录；运行中的 bot 拒绝删除，成功删除释放配额。
- **实时目录**：`bot/changed` 推送 created/updated/deleted；`bot_create` 工具创建也走同一条事件；默认 session 重建后事件带新 session id。UI 不依赖连接时 list 或手动刷新。
- **侧边栏同构**：bot 行和 session 行共用 `SidebarRow`，共享选中/running/pending/unread/双击重命名/右键/文件夹拖放。
- **写入时机**：bot 生命周期属于产品层，可能发生在 bot 自己的回合内；owned profile 的写入使用 SettingsWriter 的 bot 专用入口，跳过全局 turn guard，但仍复用同一校验、commit lock 和 revision。
- **失败补偿**：profile 写成功但 session 或 registry 失败时，删除 profile/session；registry 写失败删除空 session；删除 bot 时 session 先删、profile 再删、记录最后删，记录失败会恢复 profile。

## 验收

阶段一完成时，至少能证明：

1. 用户从侧边栏创建 bot，重启后仍能找到它。
2. 一个 bot 能发现另一个可见 bot。
3. 一个 bot 能给另一个 bot 发消息，目标 bot 在同一个会话语义下完成自己的回合（阶段一投递不等待，调用方只拿到入队回执）。
4. 停止、取消、权限和失败行为与普通 session 一致。
5. 普通 session 看不到、也不能调用 bot 工具集。
6. bot 的创建和消息链路没有绕过 `SessionService`。
7. bot 之间不会因为递归创建或无边界发现形成失控循环。
8. 用户愿意日常使用它，而不是只跑一次演示。
9. bot 有独立 owned profile，编辑它不影响其他 bot。
10. 侧边栏 bot 行与 session 行同构，支持文件夹和右键；bot 的创建、编辑、删除和 bot 创建的 bot 都实时可见。

## 退出条件

- Bot 能作为内部产品稳定使用。
- Bot 产品层与 session 核心边界清楚。
- 出现的问题主要在 bot 产品策略，而不是会话语义或写入路径。
- 下一阶段可以在不改动 bot 内核的前提下扩展 bot 产品层。
