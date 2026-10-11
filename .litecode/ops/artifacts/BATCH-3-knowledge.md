# Batch 3 · knowledge 实现对照

负责人：Tools。基线：`075308b`。工作区 `/workspace/litecode`，分支 `ops/team-framework`。**未 commit / 未 push。**

## 1. 节点对照表

| key | 结论 | 证据（文件 + 符号 / 测试） |
|---|---|---|
| `knowledge`（framework, pending） | **一致** | `docs/knowledge-references.md` + `src/knowledge/mod.rs`；契约展开五子均 A |
| `knowledge-ref-authority`（pending） | **一致** | KR-INV；`compute_once`；`every_case_names_a_rule_the_spec_defines` |
| `knowledge-parse`（pending） | **一致** | `parse_knowledge_markdown` / `Status`；`layout_follows_the_spec` |
| `knowledge-index`（pending） | **一致** | `start` / `snapshot` / `fresh_content` / `turn_note` |
| `knowledge-ref-resolve`（pending） | **一致** | `resolve` / `validate_with`；`resolution_follows_the_spec` |
| `knowledge-tool`（pending） | **一致** | `KnowledgeTool::{execute,check,refs,…}`；`notify_paths` |
| `knowledge-client-view`（client-ux） | **他域** | 前端已核实；本域以 `相关：` 指向，不复述 |

**统计（本域 6）：** 结论 A **6** / 结论 B **0**。

### 唯一真相钉板

| 层 | 权威 |
|---|---|
| 语法 / KR-* | `docs/knowledge-references.md` + `src/knowledge/spec_tests.rs` + `tests/fixtures/knowledge_refs/cases.json` |
| 节点内容 | `knowledge/**/*.md`（索引派生物可重建） |
| 运行时结论 | 后端 `KnowledgeIndex` / `compute_once`（前端只投影） |

## 2. 正式 issue

| id | grade/layer | status | 处置 |
|---|---|---|---|
| **ISSUE-022** | suspect / gate | **open** | KN-B2-01：准入模板「证据：」vs 惯例「依据：」双名；待 GM 是否收敛文案 |
| KN-B2-02 | — | **无** | 未发现 `src/knowledge` 实现 bug（按 §8.4） |

机读：`.litecode/ops/issues/ISSUE-022.json`。

## 3. 待删 / 待改非知识 md（本域撞车来源）

| 路径 | 建议 | 说明 |
|---|---|---|
| `docs/knowledge-references.md` | **保留（权威规范）** | 与节点互补，不是冲突副本；**不得**当待删 |
| `src/tools/descriptions/knowledge.md` | **保留** | agent 工具面指引；若与节点打架再改描述，不删 |
| `.litecode/ops/context/knowledge-node-admission.md` | **保留（ops）** | 团队流程，非产品知识库 |
| （无发现第二套「知识语法」产品 md） | — | 仓内无平行 KR 说明书可删 |

本批 **无删除执行**；上表供 GM 审。若日后规范完全迁入 `knowledge/` 节点，再议是否降级 `docs/knowledge-references.md` 为生成物——**当前不要删**。

## 4. 互引补全

- `knowledge-ref-authority` ↔ `knowledge-client-view`（已有 `相关：`）
- framework `knowledge` 契约展开五键齐全
- 本批未改主张；未改 `docs/` / `src/knowledge/*`（§8.4）

## 5. 本批改动（未提交）

- `knowledge/framework/knowledge.md` + 五个 contracts — 追加 §7.2
- `.litecode/ops/issues/ISSUE-022.json`
- （无产品代码改动）

## 6. 阻断声明

**无。**

## 7. kb_check

全库：`node .litecode/ops/tools/kb_check.mjs` → `{"ok":true,"errors":0,"warnings":76,"nodes":78}`（2026-10-12 HKT）。
本域无新增 error。`webfetch-ssrf` 对未提交的 `is_non_public_ip`/`validate_public_http_url` 有 `symbol_drift` 警告（改码相对 HEAD，预期；提交后消除）。
