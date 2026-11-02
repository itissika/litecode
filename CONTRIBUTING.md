# 贡献指南

LiteCode 的原则很简单：**把 Tool 和 Context 两件事做到极致**，不追新概念，用心写好每一处设计。

## 行为准则

保持尊重与务实。讨论技术，不讨论人。评审对事不对人。

## 开发环境

- Rust（MSVC 工具链，edition 2024）
- Node.js 22+（web / desktop）
- Linux 打包需原生 Linux 环境

## PR 流程

1. Fork 仓库，基于 `master` 建分支。
2. 提交时写清「做了什么、为什么」，规范见 [AGENTS.md](AGENTS.md)。
3. 确保本地构建通过。
4. 发起 PR，描述变更动机与影响面。

PR 会触发 Windows CI（sidecar 构建与冒烟）。评审关注：

- 是否保持 Tool/Context 的设计一致性
- 是否引入不必要的依赖或复杂度
- 安全边界（权限、路径、凭据）是否收紧而非放宽

## 参考

- 项目契约（提交、日常、路由）：[AGENTS.md](AGENTS.md)
- 发版与打包：[docs/agents/](docs/agents/)
- 架构与模块划分：[README.md](README.md)
