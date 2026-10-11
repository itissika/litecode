# Batch 3 · release-desktop

负责人：前端 · 基线 `075308b` · 日期 2026-10-12（HKT）  
范围：`release-design` / `desktop-shell` / `slim-packaging` / `component-distribution` / `bundle-resolve`（本域 Batch 2 从零钉下，无更早 enabled 存量）。

## 1. 节点对照表

| key | 状态 | 结论 | 证据 |
|---|---|---|---|
| `release-design` | pending | **一致，无实质漂移** | Windows 唯一瘦 NSIS；大资源按需本仓 Releases；不用 CDN。`desktop/package.json` win target 仅 `nsis`；`package_win.ps1` 默认 slim；CI `windows-signed-release.yml` 上传 components。无 portable 发布形态（旧 Release 资产残留不改变当前默认）。 |
| `desktop-shell` | pending | **一致，无实质漂移** | Electron `main.ts` / `sidecar.ts` 托管进程与 IPC；`ComponentManager` 下载；`litecodeBundlesDir` → `%LOCALAPPDATA%/litecode/bundles`。壳不持会话/布局真源。 |
| `slim-packaging` | pending | **一致，无实质漂移** | `scripts/package_win.ps1` 默认 slim（`-Full` 才嵌模型+linux）；`assemble_product` / `desktop/package.json` `extraResources` 仅 `sidecar`（无 models/linux）；DirectML 留包内与节点准入说明一致。 |
| `component-distribution` | pending | **一致，无实质漂移** | `desktop/src/components.ts`：`parseManifest` / `releaseDownloadPrefix` / `ComponentManager`；URL 前缀本仓 Releases；sha256 校验；`LITECODE_COMPONENTS_BASE` 离线覆盖；`scripts/build_components.mjs` + CI 上传。**v0.2.3** Release 已含 `litecode-components.json` + embed/linux tar（API 核实）。 |
| `bundle-resolve` | pending | **一致，无实质漂移** | `resolveBundledModelDir`：env → bundles → sidecar → repo；`devRepoRoot(isPackaged)` packaged 时 `undefined`；Rust `source_tree_model_path` 仅 `cfg(debug_assertions)`（`embed.rs`，ide-substrate 联查）。 |

**统计：** 一致 5 / 不能扩写 0。无 B2。

## 2. 正式 issue

| id | grade | layer | status | 由草案 | 摘要 |
|---|---|---|---|---|---|
| ISSUE-020 | suspect | env | **closed** | DRAFT-REL-1 | 当前 latest v0.2.3 已有组件资产；历史 tag 缺资产风险记一笔后关闭 |
| ISSUE-015 | drift | gate | open | DRAFT-REL-2 | 构建路径前缀卫生（无 remap）；**Andy 口径：非用户功能，不升产品** |
| ISSUE-016 | suspect | product | **wontfix** | DRAFT-REL-3 | electron-updater / 自动更新单独立项，非本批缺陷 |

机读：`.litecode/ops/issues/ISSUE-015.json`、`ISSUE-016.json`、`ISSUE-020.json`。

**REL-2 处置：** 不改产品形态、不阻断；可选后续在 CI 加 `--remap-path-prefix`（构建卫生），与 `bundle-resolve` 已落地的「release 不回落源码树」正交。

## 3. 待删 / 待改非知识 md

| 路径 | 建议 |
|---|---|
| 旧 Release 上的 `Litecode-Portable-*.exe` 资产 | 非仓内 md；文档已写「唯一 NSIS」。无需删仓内文件。 |
| `CHANGELOG.md` / `desktop/README.md` / `README.zh-CN.md` | 与节点一致，**保留**为用户文档，不是第二套知识主张。 |
| （无）与 slim/bundle 主张冲突的过时 ops 外 md 待删项 | |

## 4. 互引补全

- `bundle-resolve` 已 `相关：component-distribution`。
- `component-distribution` 已 `相关：slim-packaging`。
- 与 ide-substrate embed 发现：节点正文已说明 Rust 回退归对端；本批不新开第二真源。可选后续由 ide-substrate 在 embed 节点加 `相关：bundle-resolve`（非本域改对端）。

pending 节点已追加 §7.2 短核实结论。

## 5. 阻断声明

**无。**

## 6. kb_check

五节点 `--node` 均为 `errors=0 warnings=0`。

## 7. 本轮未改产品代码

核实 + issue 落盘 + 节点短结论；不 commit/push。
