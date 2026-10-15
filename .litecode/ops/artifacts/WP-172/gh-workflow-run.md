# 中层：如何手动跑重型 GitHub Actions

仓库：`itissika/litecode`  
需要：本机已登录 `gh`（或环境变量 `GH_TOKEN` 有 `actions:write` + `contents:read`）。

## 原则

1. **默认走 Actions**，不要在云电脑上抢独占槽跑 `cargo test` / 全量 vitest。
2. 云电脑门禁仍保留兜底：`MemAvailable ≥ 2GiB` 才开重型，跌破 **1GiB** 看门狗杀掉（`LC_MIN_MEM_GIB` / `LC_KILL_MEM_GIB`）。只在 Actions 不可用或 GM 明确要求时用。
3. 推送用**独立分支**；不要为了试跑去改 workflow 的 `on:` 加 push/PR。

## 1. 准备分支

```bash
# 在已包含要验的改动的提交上
git checkout -b ops/wp-NNN-verify   # 或沿用已有验证分支
git push -u origin HEAD
```

若改动还在别人的工作区、未提交：先让 owner 提交到独立分支，再跑。

## 2. 触发流水线

### A. 日常重型（cargo + web + desktop）

```bash
gh workflow run ci-linux.yml \
  -R itissika/litecode \
  --ref <branch> \
  -f note="WP-NNN mid verify"
```

### B. 干净环境 / 工具链验收（init_env 全流程）

```bash
gh workflow run ci-init-env.yml \
  -R itissika/litecode \
  --ref <branch> \
  -f note="WP-NNN init_env" \
  -f skip_heavy=false
```

只测安装与工具链、不跑编译测试时：`-f skip_heavy=true`。

## 3. 跟 run、拿日志

```bash
# 列出刚触发的 run
gh run list -R itissika/litecode --workflow=ci-linux.yml --branch <branch> --limit 5

# 跟日志（把 RUN_ID 换成上一步的 databaseId）
gh run watch <RUN_ID> -R itissika/litecode

# 下载 artifact（日志目录）
gh run download <RUN_ID> -R itissika/litecode -n ci-linux-logs-<RUN_ID> -D .tmp/ci-linux-logs/
# init_env 对应 artifact 名：ci-init-env-logs-<RUN_ID>
```

浏览器：`https://github.com/itissika/litecode/actions` → 对应 workflow → 本次 run。

## 4. 写回工作包

验收产物里至少记：

- run URL（`https://github.com/itissika/litecode/actions/runs/<RUN_ID>`）
- 结论：pass / fail
- 若 fail：`summary.txt` 或 `test_failures.txt` 里的步骤名，并按域派给对应 bot（产品代码仍由域 owner 改，工具集管家不改产品代码）

## 5. 工作流文件名对照

| 文件 | 用途 | 触发 |
|---|---|---|
| `ci-linux.yml` | 串行 cargo build/test --no-fail-fast + web tsc/vitest + desktop test；`CARGO_BUILD_JOBS=2`；vitest `--maxWorkers=2` | **仅** workflow_dispatch |
| `ci-init-env.yml` | 干净 runner 跑 `init_env.sh` | **仅** workflow_dispatch |
| `windows-desktop.yml` | Windows 打包 | **仅** workflow_dispatch |
| `windows-sidecar.yml` | Windows sidecar（WP-172 去掉 push/PR） | **仅** workflow_dispatch |
| `windows-signed-release.yml` | tag→release | tag/release **自动保留** |

### vitest workers

| 环境 | 命令 | 原因 |
|---|---|---|
| GitHub Actions（`ci-linux.yml`） | `vitest run --maxWorkers=2` | runner 内存更宽裕 |
| 云电脑验收（`init_env.sh` / WP-171） | `vitest run --maxWorkers=1` | 与 2GiB/1GiB 门禁配套，避免挤爆 |

## 6. 常见问题

- **`could not resolve workflow`**：分支上还没有该 yml，先把 `.github/workflows/ci-*.yml` 推进该 `--ref`。
- **`init_env.sh` missing**：`ci-init-env` 需要 WP-171 的 `init_env.sh` 已在该 ref；先确认 `git ls-tree` 里有该文件。
- **云电脑仍想跑重型**：向后端中层要独占槽，并遵守 2GiB 开跑 / 1GiB 杀线；优先改走本页 Actions。
