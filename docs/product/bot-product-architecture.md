# Bot 产品与接入架构

## 1. 背景

LiteCode 已经有统一的会话业务面：UI、subagent、后台自动回合都通过 `SessionService` 消费同一套会话语义，`SessionManager` 只保留预占、锁和写入原语。

下一步不是继续拆会话内核，而是在这个内核上长产品面：

- 对内：把 bot 做成 LiteCode 的一等产品能力，让内部 agent 先吃到统一能力。
- 对外：通过 ACP 和飞书两个官方适配器接入外部使用场景。

这两条线共用同一套会话语义，但不能互相污染。Bot 是产品身份层，ACP 和飞书是接入层，`SessionService` 是核心层。

## 2. 目标产品形态

### 2.1 Bot

Bot 是 LiteCode 面向人和 agent 的智能体产品对象，不是一个新 session 类型。

- Bot 有稳定身份：名称、归属、可见性、能力、绑定关系。
- Bot 有至少一个默认 session；后续可以拥有多个 session。
- Bot 的会话仍然是普通 session，消费同一套 turn、事件、权限、取消和历史。
- Bot 多一套受控工具集：创建 bot、发现 bot、给 bot 发消息。
- Bot 工具集只对 bot 会话注入，普通 session 不获得这些能力。
- Bot 之间可以互相发现和通信，但发现范围、可见性、配额和递归创建必须受产品策略约束。

MVP 从侧边栏的“新建 bot”开始，先让 bot 成为真正可用的内部产品，而不是只做演示壳。

### 2.2 对内

内部消费是 bot 的第一验证场：

- LiteCode 自己的 agent 先使用 bot 工具集。
- bot 之间的消息、发现和创建走同一套 session 服务。
- 普通 session、subagent 和 bot 共享会话语义，但工具面和产品权限不同。
- 内部验证证明产品语义和工具策略成立，不替代外部契约验证。

### 2.3 对外

- **ACP**：面向编辑器的官方适配器，只接普通 session，不碰 bot。
- **飞书**：面向 IM 的官方适配器，绑定即 bot；入站消息路由到 bot 的 session，不创建普通 session。

ACP 解决编辑器和本地受信客户端；飞书解决远程、异步、多用户和消息平台语义。两者都不能把各自的协议字段带进 session 内核。

## 3. 架构

```text
                   ┌─ Web UI（内部特权适配器）
                   ├─ ACP adapter（普通 session）
Session API ───────┼─ Feishu adapter（绑定 bot）
                   └─ 本地脚本 / SDK

Session API
    ↓
SessionService（会话业务面）
    ↓
SessionManager（预占、锁、写入）
    ↓
Session 数据与事件
```

### 3.1 核心层

`SessionService` 负责会话语义：

- 生命周期：创建、列表、打开、删除；
- 对话与回合：发消息、排队、启动、停止；
- 权限与交互；
- 观测与历史；
- 回退、手动压缩等会话动作。

核心不知道 bot、ACP 或飞书的协议细节。

### 3.2 Bot 产品层

Bot 产品层负责：

- bot 身份与归属；
- bot 目录和发现；
- bot 工具集；
- bot 之间的通信策略；
- 绑定关系。

Bot 产品层调用 `SessionService`，不直接写 session 存储，不拥有第二套 runtime。

### 3.3 接入层

- **Web UI**：内部、可信、特权适配器，可以使用展示和控制扩展，但会话动作必须走 service。
- **ACP**：薄适配器，翻译编辑器和 session 之间的协议；只暴露普通 session。
- **飞书**：厚适配器，拥有 webhook、消息、卡片、chat/thread、幂等、重试和 bot 绑定；不把飞书字段带进核心。

## 4. 推进阶段

### 阶段一：Bot 内部一等能力

先把 bot 做成 LiteCode 内部真实可用的一等产品：

- bot 身份和默认 session；
- 侧边栏新建入口；
- bot 目录和最小发现；
- bot 工具集的受控注入；
- bot-to-bot 消息；
- 普通 session 与 bot 的权限隔离。

这一阶段不接 ACP，不接飞书，不引入远程身份和多租户。

### 阶段二：对外薄契约与 ACP

在 bot 内部稳定后，做一次薄对外验证：

- 定义协议中立的 session API 子集；
- 做 ACP 适配器；
- 只接普通 session；
- 验证流式、权限、取消、恢复和客户端生命周期。

ACP 不感知 bot，也不改变 bot 产品层。

### 阶段三：飞书 bot 接入

- 飞书绑定即 bot；
- 适配器负责 chat/thread、消息、卡片、身份、幂等和重试；
- bot 绑定关系由产品层拥有；
- 飞书不进入 session 核心。

阶段顺序的默认判断是：bot 内部先行，ACP 作为薄外部验证，飞书作为厚外部验证。若产品节奏要求飞书提前，可以调整，但不能跳过核心契约和 bot 身份的稳定化。

## 5. 做什么，不做什么

### 做

- Bot 作为产品身份，session 作为运行对象。
- 一个 bot 先有一个默认 session，但保留未来多 session 的身份模型。
- Bot 工具集只注入 bot 会话。
- Bot-to-bot 消息复用普通会话的队列和回合。
- ACP 只接普通 session。
- 飞书绑定 bot，不创建普通 session。
- 所有会话动作通过 `SessionService`。
- 适配器独立构建、独立部署、可替换。

### 不做

- 不把 bot 做成新的 session 内核或新的 runtime。
- 不新增第二套 session store、writer 或消息总线。
- 不把 bot 字段、飞书 chat_id、ACP 方法名写进核心。
- 不让普通 session 获得 bot 工具集。
- 不让 ACP 暴露 bot 目录或 bot 工具。
- 不让飞书适配器直接操作 session 存储。
- 不在第一阶段做全局 bot 网络、市场、跨租户发现。
- 不用 `SubagentHub` 代替 bot 产品层的发现和通信关系。

## 6. 验收与成功信号

Bot 阶段成功不是“接口能返回”，而是：

- 用户能从侧边栏创建 bot，并持续使用；
- bot 能发现和给另一个 bot 发消息；
- bot-to-bot 回合、权限、取消、失败行为与普通 session 一致；
- 普通 session 不受 bot 工具影响；
- bot 的身份、会话和绑定关系在重启后仍然成立；
- 代码层没有绕过 `SessionService` 的会话写入路径；
- 架构守卫能够阻止 bot、ACP、飞书语义渗入核心。

外部阶段的成功信号：

- ACP 客户端能完成普通 session 的完整链路；
- 飞书能把消息稳定绑定到 bot，并保持幂等、重连和权限语义；
- 两个适配器不改变核心会话语义，也不产生第二套状态。

## 7. 一句话

**Bot 是 LiteCode 的产品身份层，不是 SessionService 的第二种内核；ACP 和飞书是它的官方接入面，不是核心本身。**
