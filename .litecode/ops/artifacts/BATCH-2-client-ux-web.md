# Batch 2 · client-ux（web/桌面交互部分）

负责人：前端 · 基线 HEAD `075308b` · 日期 2026-10-11  
范围：`framework/ux`、`design/ux-design`、`ux/*`、相关 contracts；**不含** `client_protocol`/`serve` 主实现（归 Session；存量 `client-projection` 保留为投影契约主节点）。

## 1. 拟新增节点

| key | 上层 | 准入 | 一句话承诺 |
|---|---|---|---|
| `layout-ssot` | ux | A1/A3 | 布局真源只在 Dockview；业务 store 只持内容，经 workbench 单向跟从 |
| `ask-surface` | ux-interaction | A2/A4 | 人机协作卡统一 PermissionCard（permission/approval/ask_user），回执仍经 agent/permission |
| `theme-tokens` | ux | A1 | 视觉单一来源：`--_dk-*` 与 glassFill，不散落硬编码 |
| `knowledge-client-view` | ux | A3 | 知识状态以后端快照为唯一权威；本地解析只服务即时预览，打开面板不写盘 |

路径：
- `knowledge/contracts/{layout-ssot,ask-surface,knowledge-client-view}.md`
- `knowledge/ux/theme-tokens.md`

全部 `status: pending`，已带 `domain: client-ux` 与 `基线：075308b`。

## 2. 拟保留 / 拟改的存量

| 节点 | 处置 |
|---|---|
| `ux-design` | 保留；补 `domain: client-ux`、`基线：075308b`（意图不动） |
| `ux` | 保留；补 domain/基线；挂 `契约展开`→layout-ssot、knowledge-client-view；`框架展开`→theme-tokens |
| `ux-interaction` | 保留；补 domain/基线；挂 `契约展开`→ask-surface |
| `client-projection` | **保留为主节点**（web `messageStore` + `client_protocol` 证据）；补 domain/基线。协议侧细节 Batch 3 与 Session 对齐，不新开第二主节点 |

未改主张措辞。

## 3. 疑似主节点重复

- 本域新增之间：无共享符号证据冲突。
- `client-projection` 与 harness-core 会话真源：`client-projection` 主张的是**投影纪律**，不是 session 行真源；与 `session-model` 不重复（相关引用即可）。
- `knowledge-client-view` vs knowledge 域新建的索引节点：本节点只承诺**客户端投影/不写盘**；解析权威归 knowledge 域。kb_check 未对本节点报 dup。

## 4. Issue 草稿（Batch 3 再落盘）

| 草案 | 级别 | 摘要 |
|---|---|---|
| DRAFT-CUX-1 | suspect / product | 多 session 下单个 Ask 会串行化整条 WS 环（`connection.rs` permission wait 排队异会话 RPC）；表现：一张 plan 卡锁死其它 session 发消息/revert。前端卡本身按 session 作用域。需 Session 域确认是否改主环。 |
| DRAFT-CUX-2 | drift / gate | ISSUE-008 已记：PermissionModal 测试样式断言过期。 |
| DRAFT-CUX-3 | drift / gate | ISSUE-001：client_protocol 权限投影测试未跟上 `free_text`/`kind`/`multi_select`。偏 Session/client_protocol，Batch 3 联查。 |

## 5. kb_check（本域新增 4 节点）

逐节点 `--node`：**errors=0 warnings=0**（layout-ssot / ask-surface / theme-tokens / knowledge-client-view）。

全库另有 1 个 error（`llm-provider-surface` 缺符号，属 model-config，非本域）。enabled 父节点引用 pending 子节点产生 `inactive_target` warning，符合 KR-NODE-3，预期内。
