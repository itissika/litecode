```node
node : bot
status : pending
summary : Bot 是产品身份层：1:1 绑定一条普通 session，记录是唯一正文权威；平台段由引擎只读注入，消息信封以 id 为准。
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

## 位置与可见性

- 文件夹是纯整理，不影响可见性。
- 位置唯一，沿树继承；组织关系生效后，后续切片负责树形渲染、可见性和防环。
- 当前不承诺组织可见性范围；平台段也不写未实现的可见性能力。

## 平台固定段

- 平台固定段由引擎拼接，位置在 harness → 正文 → bot 平台段 → citations → AGENTS.md/CLAUDE.md。
- 只注入 bot 自己的 session，不进 subagent 子会话，不进普通 session。
- 只读、不可编辑，不进 bot 正文。
- 平台段说明：bot 身份、来源信封、回复发送者用 bot_send、bot_list/bot_send 当前实际范围、bot_create 归名下、bot_send 只保证入队、任务完成即停。

## 消息来源

- bot-to-bot 消息复用普通消息、队列和回合，没有第二套消息总线。
- 信封格式以 id 为权威：`[from bot id={sender_id} name={sender_name}]`；name 是发送时快照。
- bot_send 目标优先 id；用名字必须唯一，歧义时报错，不做 first-match。
- bot_send 只保证入队，不等待目标回合结果；普通输出不会回到发送者。
