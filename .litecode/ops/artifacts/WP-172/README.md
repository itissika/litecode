# WP-172：重型校验迁到 GitHub Actions

owner：tools_steward · approver：gm · status：in_progress（草案，未 push、未触发）

## 目标

把 cargo / web / desktop 等重型校验从云电脑迁到 GitHub Actions；云电脑只做改代码和轻量检查（`validate_all`、文档、小修）。

## 硬约束（Andy）

- 新加的流水线**只能** `workflow_dispatch` 手动触发。
- **禁止** `on.push` / `on.pull_request` 自动全量跑。
- 现有 tag→release（`windows-signed-release.yml`）可保持自动。

## 交付物（本机草稿）

| 路径 | 说明 |
|---|---|
| `.github/workflows/ci-linux.yml` | ubuntu 串行；`CARGO_BUILD_JOBS=2`；vitest `--maxWorkers=2`；日志 artifact |
| `.github/workflows/ci-init-env.yml` | 干净 runner 跑 `init_env.sh`；可选 `skip_heavy` |
| `.github/workflows/windows-sidecar.yml` | **WP-172**：去掉 push/PR，仅 `workflow_dispatch` |
| `.litecode/ops/artifacts/WP-172/gh-workflow-run.md` | 中层如何 `gh workflow run` |
| `.litecode/ops/artifacts/WP-172/gm-answers.md` | GM 四条批复 |
| `board.md` | Actions 默认重型入口；云电脑 2GiB/1GiB 门禁兜底 |

## 未做（等 WP-171 run6 + GM 确认 push）

- **未 push**、**未** `gh workflow run`。push 前必须再问 GM。
- 首次分支 `ops/wp-172-actions-ci` 计划带上 WP-171 的 `init_env.sh` + 门禁改动；等 run6 结果后再申请。
- `ci-init-env.yml` 依赖该分支上已有 `init_env.sh`。

## 建议临时分支名（稍后 push 用）

`ops/wp-172-actions-ci`（勿推 main；push 须 GM 点头）
