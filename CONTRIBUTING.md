# 贡献指南

感谢你考虑为 LiteCode 做贡献。项目原则很简单：**把 Tool 和 Context 两件事做到极致**，不追新概念，用心写好每一处设计。

## 行为准则

保持尊重与务实。讨论技术，不讨论人。评审对事不对人。

## 开发环境

- Rust（MSVC 工具链，edition 2024）
- Node.js 22+（web / desktop）
- Linux 打包需原生 Linux 环境

## 构建与测试

```bash
# Rust
cargo check          # 快速类型检查
cargo test           # 全部测试

# Web
cd web && npm install && npm run build

# Desktop
cd desktop && npm install && npm run test
```

提交前请确保 `cargo check` 与 `cargo test` 通过，且不引入无关改动。

## 提交规范

所有提交必须遵守 [Agent.md](Agent.md) 中的「提交铁律」，要点：

- Conventional Commits：`type(scope): 描述`
- 一次提交只做一件事（原子提交）
- 禁止提交密钥、token、真实邮箱、大文件或编译产物

## 提交 PR 流程

1. Fork 仓库，基于 `master` 建分支。
2. 提交时遵守上述规范，写清「做了什么、为什么」。
3. 确保本地测试通过。
4. 发起 PR，描述变更动机与影响面。

PR 会触发 Windows CI（sidecar 构建与冒烟）。评审关注：

- 是否保持 Tool/Context 的设计一致性
- 是否引入不必要的依赖或复杂度
- 安全边界（权限、路径、凭据）是否收紧而非放宽

## 参考

- 系统提示词与项目契约：[Agent.md](Agent.md)
- 架构与模块划分：[README.md](README.md#项目结构)
