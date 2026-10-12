# WP-066 desktop 实施记录（ISSUE-015 release path remap）

- 包：WP-066（owner `frontend_desktop`，父包 WP-065 / WP-063）
- Issue：**ISSUE-015**（release 构建卫生：第三方 panic 路径尽量不带构建机 `.cargo/registry`）
- 分支：`ops/team-framework`
- 时间：2026-10-12 21:53 HKT（Asia/Hong_Kong）
- **未 commit**；**未 deliver**（等前端中层交叉审）

## What changed

| 文件 | 变更 |
|------|------|
| `scripts/assemble_product.sh` | 新增 `append_rustflag_remap` + `ensure_release_rustflags_remaps`；`PROFILE != debug` 时幂等追加 registry/git remap 到 `RUSTFLAGS` 并 `export`；cargo build 前打印 |
| `scripts/assemble_product.ps1` | 镜像：`Append-RustflagRemap` + `Ensure-ReleaseRustflagsRemaps`；`$Profile -ne "debug"` 时向 `$env:RUSTFLAGS` 幂等追加（Windows 路径用 Resolve-Path 反斜杠形式） |

注入内容（release only）：

- `--remap-path-prefix=<cargo_home>/registry=/rustc/registry`（sh：`/`；ps1：`\`）
- `--remap-path-prefix=<cargo_home>/git=/rustc/git`
- `cargo_home` = `$CARGO_HOME` / `$env:CARGO_HOME`，否则 `~/.cargo` / `%USERPROFILE%\.cargo`；经 `realpath -m` / `Resolve-Path` 规范为绝对路径

幂等：已含**完全相同** remap token 则不重复；保留调用方已有 `RUSTFLAGS`（空格追加）。

### 未改（禁区 / 范围外）

- `serve.sh` 内裸 `cargo build --release`（开发路径，方案明确不做）
- `web/`、后端 `src/`、产品 UI、`serve_win.ps1`（若存在）
- 无新测试文件；无 commit

## Chain map（谁走 assemble）

| 入口 | 是否经 assemble | 备注 |
|------|-----------------|------|
| `scripts/package_linux.sh` | 是 → `assemble_product.sh` | Linux 产品包 |
| `scripts/package_win.ps1` | 是 → `assemble_product.ps1` | Windows 产品包 |
| `scripts/package_local.ps1` | 是 | 本地包 |
| `scripts/package_local_cuda.ps1` | 是 | CUDA 本地包 |
| `scripts/dev_win.ps1` | 是（可 `-SkipAssemble`） | 开发；走 assemble 时 Profile 非 debug 即获 remap |
| `.github/workflows/windows-sidecar.yml` | 是 → 直接 `assemble_product.ps1` | CI sidecar |
| `.github/workflows/windows-signed-release.yml` | 是 → `package_win.ps1` → assemble | official channel |
| `scripts/smoke_product.{sh,ps1}` | 消费 assemble 产物 | 不自行 cargo |
| `scripts/serve.sh` | **否**（裸 `cargo build --release`） | **故意不改**（开发诊断路径） |

**链缺口**：仅 `serve.sh` 的开发 release 旁路；无其它 packaging 旁路跳过 assemble。无需改 workflow yml（一律经 assemble）。

## Evidence A（必做）— RUSTFLAGS 干跑

从真实脚本抽取 helper 后调用（本机 `CARGO_HOME=/workspace/litecode/.litecode/ops/.toolchain/cargo`）：

```bash
cd /workspace/litecode
eval "$(sed -n '/^append_rustflag_remap()/,/^}/p; /^ensure_release_rustflags_remaps()/,/^}/p' scripts/assemble_product.sh)"
```

| 场景 | 结果 |
|------|------|
| `PROFILE=release`，空 `RUSTFLAGS` | 注入两条 remap（registry + git → `/rustc/...`） |
| release 再调一次 | **idempotent: OK**（不重复） |
| release + 已有 `-C link-arg=...` | **preserve: OK**（前缀保留，后追加 remap） |
| `PROFILE=debug`，空 | **debug empty: OK**（不注入） |
| debug + 已有 `-C debuginfo=2` | **debug preserve: OK**（原样保留、无 remap） |

干跑输出摘录：

```
RUSTFLAGS=[--remap-path-prefix=/workspace/litecode/.litecode/ops/.toolchain/cargo/registry=/rustc/registry --remap-path-prefix=/workspace/litecode/.litecode/ops/.toolchain/cargo/git=/rustc/git]
...
idempotent: OK
preserve: OK
debug empty: OK
debug preserve: OK
```

另：`bash -n scripts/assemble_product.sh` → OK。箱内无 `pwsh`；`.ps1` 为逻辑镜像（Join-Path / Resolve-Path + token 幂等），未做运行时解析。

## Evidence B（可选）— strings 抽查

- `target/release/litecode`：**不存在**（仅有 `target/debug/litecode`，~594MB，2026-10-12 20:45 HKT）
- 完整 `assemble_product` / `cargo build --release` **未跑**（含 web build + 全量 crate，过重；本包以 A 为准）
- 建议后续：CI / GM 在 release assemble 产物上 `strings litecode | rg '\.cargo/registry'` 抽测应为无本机构建机前缀（应为 `/rustc/registry` 等）

## npm test

仅改 `scripts/`，desktop 单测面无交。未强制重跑；上一包基线 WP-044/046 为 **127 pass**。本包判定：**N/A**（构建脚本卫生，非 TS）。

## Diff 摘要

```
 scripts/assemble_product.ps1 | +35
 scripts/assemble_product.sh  | +29
 2 files changed, 64 insertions(+)
```

另新增本文件 `WP-063-desktop.md`。

## Success checklist

- [x] release assemble（sh+ps1）仅 release 注入 remap
- [x] 幂等 + 保留已有 RUSTFLAGS
- [x] 证据 A 齐（release 有 / debug 无）
- [x] 证据 B 环境限制已记
- [x] 链图文档化；serve.sh 旁路不改
- [x] 未 commit
