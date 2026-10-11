# Batch 2 · client-ux（协议投影 / serve）目录清单

负责人：Session（协议侧）。基线核对：`075308b`。  
范围：`src/client_protocol/**`、`src/serve/**`（认证/绑定/ws Ask 登记）。**不含** web 布局/主题/知识面板（前端已建 layout-ssot / theme-tokens / knowledge-client-view / ask-surface）。

## 1. 拟新增节点

| key | 路径 | 上层 | A# | 一句话承诺 |
|---|---|---|---|---|
| `serve-bind-auth` | `knowledge/contracts/client/serve-bind-auth.md` | `ux` | A6 | 非回环必须认证；/api 只认 Bearer；/ws 可 query token；唯一 serve 安全门（B3） |
| `connection-ask-registry` | `knowledge/contracts/client/connection-ask-registry.md` | `client-projection` | A2/A5 | Ask 阻塞 agent 不阻塞连接；request_id→reply_tx；断线 Deny；未知 grant 不静默丢 |

均已落盘，`status: pending`，`domain: client-ux`，`基线：075308b`。`kb_check --node` 对本二节点 errors=0 warnings=0。

## 2. 拟保留 / 拟改存量

| 节点 | 处置 |
|---|---|
| `client-projection` | **保留**主张（镜像/双游标/乐观/重连）。已有 domain/基线；补契约展开 → `connection-ask-registry`。 |
| `ux` | **保留**。补契约展开挂 `serve-bind-auth`（与前端已挂的 layout/knowledge 并列）。 |
| `ask-surface`（前端 pending） | **保留**为 UI 表面主节点；本域 `connection-ask-registry` 用 `相关：` 指向它，不复述 kind/卡片。 |
| `layout-ssot` / `knowledge-client-view` / `theme-tokens` | 前端负责，本清单不改。 |

## 3. 疑似主节点重复

- `connection-ask-registry` vs `ask-surface`：前者是连接循环/回执登记，后者是 PermissionCard 与 kind；主张不同，已交叉引用。
- `connection-ask-registry` vs Tools `tool-authorization`：授权求值在管线，wire 配对在连接；本节点不写 floor/规则。
- `serve-bind-auth` 与投影节点无共享符号证据；`kb_check` 无对本二节点的 dup 警告。
- 前端 `ask-surface` / `theme-tokens` 曾报 `missing_symbol`（全库曾 errors=3）；当前全库 `kb_check` **errors=0**（若前端已修符号链则无需本域跟进；若复发归前端）。

## 4. Issue 草稿

| 关联 | 说明 |
|---|---|
| **ISSUE-001**（种子，`drift` / gate / client-ux / core=client-projection） | `tests/client_protocol_project.rs` 等未跟上 PermissionAsk 新字段，测试二进制不跑。属守护缺口；Batch 3 修测试，本轮不改测试。 |
| **ISSUE-008**（种子，gate / client-ux） | PermissionModal 样式断言过期；归前端。 |
| （无新开 B3） | `serve-bind-auth` 主张与 `tests/serve_bind_auth.rs` 矩阵一致；未发现绕过唯一安全门的 bug。 |

## 5. 刻意不建

- 独立「ws 协议方法清单」：属枚举，排除项。
- 第二套 SessionSnapshot 节点：并入既有 `client-projection`。
- CLI 入口：非本轮协议投影重点。
