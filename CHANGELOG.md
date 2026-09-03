# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式，版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

## [0.1.7] - 2026-09-03

### 新增

- edit 一次调用可提交多处独立替换；匹配忽略换行与常见字形差异，失败块给出结构化反馈。
- grep 按 token 预算自动从展开片段降到附近上下文或匹配行；输出模式明确为 lines / files / content 三档。
- 压缩点可点开查看摘要。
- LSP 诊断改为流式返回，并支持增量同步。
- 文件树支持 glob 过滤与定位展开。
- 初始化工作区时自动生成 `.litecode/` 目录说明 README。
- 工作区 MCP / 自定义工具定义文件变更时自动刷新，无需重启。
- 提交回执附带最后一条用户消息预览。
- 界面引入毛玻璃（ShapeBlur）视觉组件。

### 变更

- Session 搜索由双栏翻页改为统一排序结果。
- 上下文压缩提示简化，不再输出 reasoning。
- 搜索过滤与路径排除逻辑统一，gitignore 规则处理一致。
- 设置面板各分节独立保存，草稿状态更可靠。

### 修复

- LSP 诊断按文档版本缓存，过期 Error 不再回填。
- 工具写完后迟到取消不再覆盖真实结果。
- 未钉在底部时，流式增长和折叠不再把阅读位置顶走。
- LLM HTTP 错误附带请求规模，便于区分超大体与网络故障。
- 不可读 / 被锁文件不再中断语义搜索。
- 代码搜索索引在 excludes / gitignore 变更时自动同步。
- 文本索引路径追踪与对账，避免不必要重建。
- 设置持久化跳过不完整的未保存行。

## [0.1.6] - 2026-08-25

### 新增

- 终端多开时右键可关闭并杀进程。
- gitignore 开关拆分：文件浏览与语义检索可独立控制忽略规则。

### 变更

- README 特性介绍图改用 Full.webp 动图。

### 修复

- LSP 移除 hub 全局互斥锁：并发请求不再互相阻塞，并支持请求取消。

## [0.1.5] - 2026-08-24

### 新增

- Session 以 seq 为权威行号（wire、持久化、Revert、压缩、搜索与 UI 全链路）。
- MCP：工具元数据、allowlist、服务器管理；OpenCode 适配器。
- Ark Coding Plan 适配。
- 会话 active plan 与 UI 指示。
- 桌面 Hub 首页重设计。
- Git 面板：提交图、树形浏览与多选。
- 终端 bash jobs 与工作区控制；设置项自动保存。
- 上下文占用环分项展示（system / 工具 schema / 调用 / 输出 / 对话）。
- 安装包内嵌 embed 与 slim Linux tar；Open Remote 分传模型与服务端包。

### 变更

- 工具设置改为按 Agent 绑定卡片，移除全局 Tool Catalog；MCP / 自定义工具支持工作区作用域。
- grep 默认仅返回匹配行；可选 `expand` 展开代码片段。
- read 参数改为 `start_line` / `end_line`（取代 offset / limit）。
- `.litecode/excludes.json` 变更时自动热重载。
- 移除 hook 系统。
- LLM SSE 长连接取消 120s 超时。

### 修复

- LSP 在 Windows 上解析 npm shim。

## [0.1.4] - 2026-08-16

### 修复

- 运行中 Revert 会中断当前 turn，已截断的日志不会被未提交 delta 或 interrupted 工具输出写回去。
- 自动压缩在日志已被截短时中止，避免把已删除内容写进 checkpoint。
- 前端在 Revert 后丢掉未封口的流式 overlay，并忽略迟到的 `turn_finished`，避免新 turn 被旧结束事件打回 idle。

## [0.1.3] - 2026-08-16

首次 GitHub 公开版本。

### 变更

- README 英文化，中文版本保留为 `README.zh-CN.md`。
- 特性展示改用 4 张方版 WebP 动图（轻量 / 搜索 / Session 并行 / 运行中切换）。

## [0.1.2] - 2026-08-15

首次对外公开的版本。

### 新增

- 极致轻量的运行时：静态内存 ~50M，峰值 ~100M。
- Session 后台并行 + 并发子代理（不可嵌套）。
- OpenAI Responses 一等公民：内部唯一权威格式，openai / deepseek / mimo 适配器。
- 桌面端（Electron）与浏览器端双形态，本地 sidecar 管理。
- 远程工作区：SSH 部署 Linux server + 隧道回连。
- 语义搜索引擎：tantivy + usearch（ANN）+ tree-sitter 结构化命中。
- 工具集：grep / write / webfetch / subagent 等，含自定义工具示例。
- 权限系统与敏感路径防护。
- 嵌入模型随仓分发（granite-embedding）。
