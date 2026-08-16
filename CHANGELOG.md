# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 格式，版本号遵循 [SemVer](https://semver.org/lang/zh-CN/)。

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
