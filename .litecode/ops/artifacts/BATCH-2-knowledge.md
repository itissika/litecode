# Batch 2 · knowledge 目录清单

基线：`075308b`。负责人：Tools。规范权威仍是 `docs/knowledge-references.md`（本轮不改 docs / `src/knowledge/*`）。

## 1. 拟新增节点

| key | 上层 | 准入 | 一句话承诺 |
|---|---|---|---|
| knowledge（framework） | product-brief | A3 | 关系可验证；磁盘权威；后端索引唯一真相 |
| knowledge-ref-authority | knowledge | A1/A3 | KR-INV：磁盘为准、后端唯一、前端不自判 |
| knowledge-parse | knowledge | A1 | 声明块解析；文件名主干 = 键 |
| knowledge-index | knowledge | A3 | 每工作区常驻索引；多入口同 Content |
| knowledge-ref-resolve | knowledge | A1/A2 | 引用解析顺序与固定错误码 |
| knowledge-tool | knowledge | A4 | agent 工具读索引、写落盘再刷新，无第二套解析 |

## 2. 拟保留 / 拟改存量

- 本域 Batch 2 前 **0** 个节点；无存量可改。
- 横向：client-ux 并行新增 `knowledge-client-view`（前端只投影快照）。本域 `knowledge-ref-authority` 以 `相关` 指向它，**不复述**前端细节。
- `featured-tools`（enabled）已述 knowledge 理念；本域不改其意图，仅新建契约树承接。

## 3. 疑似主节点重复消解

- 框架节点只留 docs + `src/knowledge/mod.rs` 文件级依据，符号证据下沉到 parse/index/resolve。
- `turn_note` / `start` / `resolution_follows_the_spec` 各归 index 或 resolve，避免与 framework/authority 交叉。
- 本域 kb_check：`dup_primary_suspect` **0**。

## 4. Issue 草稿

| 草稿 id | 级别 | 摘要 | 说明 |
|---|---|---|---|
| KN-B2-01 | suspect | 准入模板写「证据：」与标签清单「依据：」双名 | 工具已接受别名；是否收敛文案待 GM |
| KN-B2-02 | — | 未发现 `src/knowledge` 语法/索引实现 bug | 按 §8.4 有问题才记；本轮无 |

无阻断。

## 5. 本域节点数

- knowledge 带 `domain`：**6**
- 相关他域：`knowledge-client-view`（client-ux）1
