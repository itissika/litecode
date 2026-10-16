# WP-172 草案备注（未 push）

## 已写文件

- `.github/workflows/ci-linux.yml` — `CARGO_BUILD_JOBS=2`；vitest `--maxWorkers=2`
- `.github/workflows/ci-init-env.yml`
- `.github/workflows/windows-sidecar.yml` — **已去掉 push/PR**，仅 `workflow_dispatch`（GM 2026-10-15）
- `.litecode/ops/artifacts/WP-172/README.md`
- `.litecode/ops/artifacts/WP-172/gh-workflow-run.md`
- `.litecode/ops/artifacts/WP-172/gm-answers.md` — GM 四条批复记录
- `.litecode/ops/tools/board.md`（内存口径澄清 + Actions 入口说明）

## 设计选择（含 GM 批复）

1. **串行 + continue-on-error + 最终 gate**：对齐 init_env；npm ci 仍 fail-fast。
2. **ci-init-env 复用 checkout**：`--dir $GITHUB_WORKSPACE`。
3. **windows-sidecar**：与新流水线一样改为仅手动；`windows-desktop` 保持手动；`windows-signed-release` 保持 tag/release 自动。
4. **首次试跑分支** `ops/wp-172-actions-ci`：稍后 push 时会带上 WP-171 的 `init_env.sh` + 门禁改动；**等 WP-171 run6 结果**，且 **push 前必须再问 GM**。
5. **ci-linux** 显式 `CARGO_BUILD_JOBS=2`（与 ci-init-env 一致）。
6. **vitest**：Actions `--maxWorkers=2`；云电脑验收（init_env）`--maxWorkers=1`（内存紧）。

## 明确未做

- 未 `git push`（须 GM 确认后再推）
- 未 `gh workflow run`
- 未跑云电脑重型（WP-171 run6）
