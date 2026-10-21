# WP-202 实现 — LSP 拆多工具 + 行列对齐并进 Code

**分支：** `feat/tool-contract-ptc`（不合 main）  
**口径：** Andy 2026-10-22 定稿。硬清；Code-only（无 Direct 双轨）。

## 目标

1. **操作集（第一刀）**：`goToDefinition`、`findReferences`、`hover`、`diagnostics`、`documentSymbol`、`workspaceSymbol`（**必带 query**）、`goToImplementation`。callHierarchy / rename 本包不做。
2. **多工具拆分**：每个操作为一个独立工具名（面干净、通用易懂）；禁止再堆单一万能 `lsp` + action bag。旧 `lsp` 工具与方言**杀掉**（或硬退化为转发且默认不暴露——优先直接删除活跃面）。
3. **定位对齐业界**：`file_path` + **1-based `line` + 1-based `character`**；**砍掉**行内 `text` 定位。`workspaceSymbol` 用 `query`（+ 可选 path 范围若底座已有），不要假塞 line/character。
4. **底座**：hub/`format.rs` 已有能力挂到 agent 面；输出继续压平（locations / symbols / hover markdown），别扔生 LSP JSON。
5. **进 Code**：`ToolSurface::Code`；无 Direct 版本。

## 验收

1. 上列多工具契约落地；旧 `text` / 单 `lsp` action 方言活跃面清零。  
2. Code-only；教法对齐行列。  
3. cargo 绿；artifacts/WP-202；push 实验分支；未合 main。

## 非目标

callHierarchy、rename、Serena name_path、合 main、前端。
