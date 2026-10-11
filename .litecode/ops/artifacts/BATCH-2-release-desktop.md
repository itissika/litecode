# Batch 2 · release-desktop

负责人：前端 · 基线 HEAD `075308b` · 日期 2026-10-11  
范围：从零钉目录。`desktop/src`、`scripts/` 打包脚本、`.github/workflows` 发布链路；**不**把 `src/engines/code_search/embed.rs` 收为本域主证据（模型查找归 ide-substrate；本域只钉壳侧 resolve 与下载）。

## 1. 拟新增节点

| key | 上层 | 准入 | 一句话承诺 |
|---|---|---|---|
| `release-design` | product-brief | A1/A7 | Windows 唯一瘦 NSIS；大资源按需从本仓 GitHub Releases 拉，不用 CDN |
| `desktop-shell` | release-design | A3 | Electron 壳托管 sidecar/远程；壳不发明产品真源 |
| `slim-packaging` | desktop-shell | A1 | 默认瘦包不内嵌模型与 Linux tar；portable 退出发布形态 |
| `component-distribution` | desktop-shell | A4/A5/A6 | `litecode-components.json` + 仅本仓 Releases URL + sha256；失败可重试且不拖垮无关功能 |
| `bundle-resolve` | desktop-shell | A3/A7 | 查找顺序 env→bundles→sidecar→repo；打包后不得回落源码树 |

路径：
- `knowledge/design/release-design.md`
- `knowledge/framework/desktop-shell.md`
- `knowledge/contracts/{slim-packaging,component-distribution,bundle-resolve}.md`

全部 `status: pending`，`domain: release-desktop`，`基线：075308b`。

存量：本域原先 **0** 节点。`product-brief` 已补 `向下展开：release-design`（意图增加发布形态展开，不改产品边界句）。

## 2. 拟保留 / 拟改

无存量产品知识节点。文档侧 `desktop/README.md`、release workflow 正文留到 Batch 3 核对，不在本批改 docs 主张。

## 3. 疑似主节点重复

- `bundle-resolve` vs ide-substrate 的 embed 模型发现：本节点钉**桌面壳路径纪律**；Rust `find_bundled_model_dir` 的 debug-only 回退由 ide-substrate 核实。相关引用足够，不开第二真源节点。
- DirectML 留在包内：写入 slim-packaging 准入说明（启动硬依赖），不单开节点。

## 4. Issue 草稿

| 草案 | 级别 | 摘要 |
|---|---|---|
| DRAFT-REL-1 | suspect | v0.2.2 Release 可能仍缺 `litecode-components.json` / 模型 tar / linux tar；瘦包客户端首次下载会失败，直到补发资产。 |
| DRAFT-REL-2 | drift / product | 第三方依赖 panic 路径仍可能带构建机 `.cargo/registry` 前缀；可用 `--remap-path-prefix` 收敛（非功能 bug）。 |
| DRAFT-REL-3 | — | 自动更新（electron-updater）**明确不在本批**：有打包无 runtime updater/feed；单独立项。 |

## 5. kb_check（本域新增 5 节点）

逐节点 `--node`：**errors=0 warnings=0**（release-design / desktop-shell / slim-packaging / component-distribution / bundle-resolve）。

`product-brief`→`release-design` 的 `inactive_target` warning 预期内。
