```node
node : bot
status : pending
summary : Bot 是产品身份层：1:1 绑定普通 session，记录唯一正文权威；org 可见性按调用者位置，审批默认拒绝、ask_user 有超时，平台段只写规则。
```

# Bot 契约（阶段一锁定，pending）

## 身份与会话

- 1 个 bot 只绑定 1 条普通 session，绑定在创建时确定。没有默认 session、备用 session、多 session。
- bot 的 session 是普通 session：turn、事件、权限、取消、历史都走既有会话语义，不新增 session 类型。
- bot 的 session 不能从普通会话面单独删除；删除 bot 才删除它的 session。session 缺失时明确报错，不在读时重建。
- bot 是产品身份，不是 session 类型。ACP 只接普通 session；飞书绑定 bot，入站消息路由到 bot 的 session。

## 唯一权威

- bot 记录是唯一权威，包含四类事实：身份（id / name / description）、正文（system_prompt / tools / allowed_subagents）、位置（无 / 文件夹 / 另一个 bot）、绑定（session id）。
- 正文只有一份，存在 bot 记录里。settings 里没有隐藏的 bot profile，也没有第二份可编辑正文。
- model 只属于 session；bot 记录不存 model。max_steps 是引擎级策略，不是 bot 配置项。

## 工具与目录

- 工作区工具目录是能力真源；bot 只在正文里存选择/覆盖，不复制目录。
- bot 专属工具由“这是 bot”和位置策略派生，不落库。
- bot_create 创建即入职：新 bot 挂在调用者位置下，正文从调用者复制，工作区配额约束。
- 删除 bot 时，直接子节点顶替它的位置（父是文件夹就进文件夹，是根就成根）。
- bot 会话的 Permission/Approval Ask 默认拒绝，不弹权限卡，工具返回 denied 后 loop 继续；Allow/Deny 规则不变，不做 per-tool 矩阵。
- bot 的 ask_user 有 60s 前台答复窗口；超时返回 no answer，loop 继续。交互回合和 auto-turn 共用同一策略，不复用已断开的 WS sink。

## 位置与可见性

- 文件夹是纯整理，不影响可见性。
- 位置唯一，沿树继承；组织关系生效后，侧边栏按父子树渲染，防环，删父后子节点顶替位置。
- 人类 UI/管理面看全量目录；bot 工具只看调用者位置：自己 + 祖先链 + 同父兄弟 + 自己的子树。根节点无父，但所有根互为平级兄弟：根能看到其他根，看不到其他根的子节点；上下互见，表亲/侄子不可见。
- bot_list 只返回该范围；bot_send 的 id/name 解析也只在该范围；越界与不存在统一报 not found，不泄露存在性。
- bot_create 仍挂在调用者下。平台段只写规则、不列名单，保证与工具范围一致。

## 平台固定段

- 平台固定段由引擎拼接，位置在 harness → 正文 → bot 平台段 → citations → AGENTS.md/CLAUDE.md。
- 只注入 bot 自己的 session，不进 subagent 子会话，不进普通 session。
- 只读、不可编辑，不进 bot 正文。
- 平台段说明：bot 身份、来源信封、回复发送者用 bot_send、bot_list/bot_send 的可见性规则、bot_create 归名下、bot_send 只保证入队、任务完成即停；只写规则不列实时名单。

## 消息来源

- bot-to-bot 消息复用普通消息、队列和回合，没有第二套消息总线。
- 信封格式以 id 为权威：`[from bot id={sender_id} name={sender_name}]`；name 是发送时快照。
- bot_send 目标优先 id；用名字必须唯一，歧义时报错，不做 first-match；解析范围受 org 可见性约束。
- bot_send 只保证入队，不等待目标回合结果；普通输出不会回到发送者。
- turn 快照：turn 构造时冻结 bot 正文、prompt 与工具集合；turn 中改 bot body/工具目录不影响当前 turn，下 turn 生效；直接改 bots.json 不热加载，重启才生效，不加实时 watcher。
