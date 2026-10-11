# Batch 2 任务简报（总经理 · 2026-10-11）

## 目标
各域负责人钉掉**本域**知识节点目录。每个节点正文只一小段：凭什么成为重要节点 + 证据在哪。**不写实现核实**（那是 Batch 3）。

## 必读
- `.litecode/ops/context/knowledge-node-admission.md`
- `.litecode/ops/context/issue-grading.md`
- `docs/knowledge-references.md`
- 工具：`node .litecode/ops/tools/kb_check.mjs`（tools_version=7）

## 规则摘要
- 工作只在 `/workspace/litecode`；不碰 Andy 本机；不推 main；群内静默；不私聊 Andy；只听总经理。
- 新节点 `status: pending`，带 `domain:`、恰好一个上层、`基线:`；用 `kb_check` 过后再算完成。
- enabled 可改（contracts 可能粗审有误；其他以意图为准只打磨措辞）；状态升降仍归 Andy。
- 死亡清单辩证看待，过时可记 issue 后删/放宽。
- 发现的问题记入 issue（按 grading）；不擅自阻断整批——阻断阈值到了只上报总经理。
- **本轮不提交不推送**，改动留在工作区；攒够量或高风险前由总经理统一安排提交。

## 交付物
每个域一份目录清单，写到 `.litecode/ops/artifacts/BATCH-2-<域id>.md`：
1. 拟新增节点列表（key、上层、准入依据 A#、一句话承诺）
2. 拟保留/拟改的存量节点
3. 疑似主节点重复项（对照 kb_check 的 dup 警告）
4. 本域发现的 issue 草稿（可先记在该文件，Batch 3 再落盘）

同时在 `knowledge/` 下创建对应 pending 节点文件（正文按准入模板 §7.1）。

完成后私信总经理，附：本域节点数、kb_check 对本域结果、artifact 路径。
