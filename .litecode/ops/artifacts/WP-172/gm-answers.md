# WP-172 GM 批复（2026-10-15）

来源：Engineering GM 对草案四问的答复。仍禁止擅自 push。

| # | 问题 | 批复 | 落地 |
|---|---|---|---|
| 1 | sidecar 是否保留 push/PR？ | **去掉** push/PR，仅 `workflow_dispatch`。desktop 保持手动；signed-release 的 tag/release 自动保持。 | 已改 `.github/workflows/windows-sidecar.yml` |
| 2 | 首次试跑分支何时推？ | 分支名 `ops/wp-172-actions-ci`，**以后** push 时带上 WP-171 的 init_env + 门禁改动；**等 run6 结果**；**push 前再问 GM**。 | 记录于本文件与 draft-notes；继续只写文件 |
| 3 | ci-linux 是否设 jobs？ | 显式 `CARGO_BUILD_JOBS=2` | 已写入 `ci-linux.yml` `jobs.linux.env` |
| 4 | Actions vitest workers？ | `--maxWorkers=2`；文档写明与云电脑验收 `--maxWorkers=1` 的差异 | yml 注释 + gh-workflow-run.md + 本表 |
