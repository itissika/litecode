# WP-044 desktop 覆盖审计（WP-046 步 1）

- 包：WP-046（owner frontend_desktop，父包 WP-044）
- 分支：`ops/team-framework`
- 时间：2026-10-12 20:15 HKT（Asia/Hong_Kong）
- 范围：只读核对；**未**改产品/测试代码；**未**开步(2)裁剪；**未** commit / deliver
- 参考：`WP-035-desktop.md`（WP-041 声称覆盖）、ISSUE-028 三道门

## 实跑基线

```bash
cd /workspace/litecode/desktop && npm test
```

| 项 | 值 |
|---|---|
| tests | **127** |
| suites | **47**（TAP top-level 50 含嵌套组） |
| pass | **127** |
| fail / skipped | **0 / 0** |
| duration | ~3.5s |

关键 WP-035/041 套件：`lifecycle-mutex`、`process-recovery`、`remote-state`、`SSH tunnel args (D1)`、`stopSidecar intentional stop (D2)`、`download UX helpers (D6/D7)`、`ComponentManager pause/stop/retry + Range (D7)`、`clearManagedRemoteIfSession (D8)`、`remote-progress` / `ensureComponentForRemote`。

---

## 覆盖绿表

判定：**已覆盖** = 实现 + 同行为单测有 `file:line` 证据；**部分覆盖** = 纯函数/辅助已测，但 `main.ts` 接线或端到端未测；**缺口** = 无自动化证据（见下节）。

### A. WP-035 / WP-041 行为

| # | 能力 | 判定 | 实现证据 | 测试证据 |
|---|---|---|---|---|
| A1 | 隧道崩 → 断开 + 限次自动重连（ServerAlive / ExitOnForwardFailure / intentionalStop） | **部分覆盖** | `ssh-session.ts:97-101` ServerAlive*+ExitOnForwardFailure；`:89-91,:711-717,:727` intentionalStop；`main.ts:723-755` `onTunnelUnexpectedExit`/`attachTunnelWatch`；`:703-741` `scheduleTunnelReconnect`+`RecoveryCounter` | **有**：`ssh-session.test.ts`「includes ServerAlive and ExitOnForwardFailure」；`process-recovery.test.ts` 限次/退避/intentional。**无**：`main` 退出→disconnect→reconnect 编排 |
| A2 | Sidecar READY 后崩溃 → 限次重启 | **部分覆盖** | `sidecar.ts:16-17,:27-32` `watchSidecarExit`；`:216` stop 先 mark；`main.ts:757-809` `attachSidecarWatch`+`sidecarRecovery` | **有**：`sidecar.test.ts`「marks intentionalStop before killing」；`process-recovery` 策略。**无**：`watchSidecarExit` 单测；**无** main 自动重启编排 |
| A3 | 生命周期单飞（Open Remote / relaunch / connectRemote） | **部分覆盖** | `lifecycle-mutex.ts:5-40`；`main.ts:144-148` mutex+两 SingleFlight；`:962,:1064` start/complete 走 flight；sidecar 路径走 `sidecarLifecycle` | **有**：`lifecycle-mutex.test.ts` 顺序/拒重入/reuse。**无**：main 挂接与「双连不留孤儿」集成测 |
| A4 | D8：`connectRemote` 失败清 `managedRemote` | **已覆盖** | `remote-state.ts:6-11`；`main.ts:944,:1126` catch 清句柄；成功后 `bindManagedRemote` | `remote-state.test.ts`「nulls when the handle still points at the failed session」 |
| A5 | 下载进度：status + percent + 网速 | **已覆盖** | `components.ts:70-91,:839-859,:1041-1045`；`preload.ts:24-28` | `components.test.ts`「emits status, percent, and bytesPerSecond while downloading」；`SpeedTracker`「tracks sliding-window speed and percent」 |
| A6 | pause / stop / retry（AbortController；pause 留 `.part`；stop 删） | **已覆盖** | `components.ts:906-940,:329-332`；IPC `main.ts:1538-1551`；`preload.ts:197-206` | `components.test.ts`「pauses (keeps .part), stops (deletes .part), and retries with Range resume」 |
| A7 | Range 续传 + 非 206 回退 + sha 校验 | **已覆盖** | `components.ts:566-600,:480-481,:551-554` | 同上 pause/retry 测 +「falls back to full re-download when server ignores Range」；既有「rejects bad hashes」 |
| A8 | 启动清扫 `.tmp` | **已覆盖** | `components.ts:383-399` `cleanupComponentTmp`；构造 `:833` | `components.test.ts`「cleans stale .part and staging under bundles/.tmp」 |
| A9 | before-quit 中止在飞下载 | **部分覆盖** | `components.ts:946-954` `abortAll`；`main.ts:1850-1858` `before-quit`→`abortAll` | **有**：`components.test.ts`「abortAll pauses in-flight downloads (before-quit path)」。**无**：Electron `before-quit` 接线测 |

### B. 既有壳流程（不变量仍否由测试守住）

| # | 能力 | 判定 | 实现 / 测试 | 是否守真不变量 |
|---|---|---|---|---|
| B1 | bundle-paths 全序 env→bundles→sidecar→repo（packaged 不回落源码树） | **已覆盖** | `bundle-paths.ts` + `bundle-paths.test.ts` 全序/成对/dev-only | 是（构造假树断言解析序） |
| B2 | slim 卫生（默认无 models / linux） | **已覆盖** | `slim-packaging.test.ts` 扫 `package.json`/`package_win.ps1`/`assemble_product.ps1`；脚本内 `throw` 防泄漏 | 是（源码级真断言；非实打 NSIS） |
| B3 | components manifest / source / manager | **已覆盖** | `components.test.ts` 默认 GH Release、拒非 GH URL、畸形清单、并发下载、坏 hash、版本刷新、缺清单可重试 | 是 |
| B4 | remote-progress 映射 + ensure 失败卸监听 | **已覆盖** | `remote-progress.ts` + `remote-progress.test.ts` | 是 |
| B5 | sidecar stop / boot 失败文案 | **已覆盖** | `sidecar.test.ts` stop + formatSidecarBootFailure | 是（停进程）；崩溃恢复见 A2 |
| B6 | SSH 凭据物化 + finally 清理 | **已覆盖** | `ssh-credential-material.test.ts` | 是 |
| B7 | ipc-trust / url-policy / window-guard | **已覆盖** | 各自 `*.test.ts` | 是 |
| B8 | ssh-session 安全 argv / SCP | **已覆盖** | `ssh-session.test.ts` safety helpers | 是 |
| B9 | BrowserHost / layout / recents / handshake 等壳周边 | **已覆盖** | 对应 `*.test.ts`（计入 127） | 是（局部契约） |

### C. scripts + workflows（只读分类）

| 资产 | 分类 | 说明 |
|---|---|---|
| `scripts/smoke_product.sh` / `.ps1` | **真断言** | READY、health、`/api/settings` 200/401、同工作区锁冲突、重启后锁释放；失败 `exit 1`/`throw`。测的是 **product/sidecar**，不是 Electron desktop 壳 |
| `scripts/package_win.ps1` slim 泄漏检查 | **真断言** | 解包后若含 `sidecar\models` 或 `linux` 则 `throw`；被 B2 静态测锁住 |
| `.github/workflows/windows-signed-release.yml` 版本三角 | **真断言** | `Cargo.toml` ≡ `desktop/package.json` ≡ tag `v*`，不匹配 `throw` |
| `.github/workflows/windows-desktop.yml` Package + Checksums | **真断言（打包）** | 跑 `package_win.ps1`、`build_components.mjs`、上传与 checksum；**从不** `cd desktop && npm test` |
| `.github/workflows/windows-sidecar.yml` models 缺失分支 | **仅日志** | `Write-Host "models not present; sidecar smoke still runs…"`，不 fail |
| 全 workflows 对 desktop 单测 | **假绿风险** | CI 可绿而本地 127 门从未在 CI 执行（建议见缺口 G1） |

---

## 覆盖缺口

| ID | 缺口 | 严重度 | 证据 / 影响 |
|---|---|---|---|
| G1 | **桌面 CI 不跑 `npm test`**（建议 only，本包不做） | 高 | `windows-desktop.yml` 无 `npm test`/`node --test`；ISSUE-028 与 WP-041 门仅本地。建议：后续独立包在 `windows-desktop` 加 `cd desktop && npm ci && npm test` 一步 |
| G2 | **隧道意外退出→disconnect→限次重连** 无自动化 | 高 | 逻辑在 `main.ts:723-741`；单测只锁 SSH 参数与 `RecoveryCounter`。手测清单仍空（见 WP-035） |
| G3 | **sidecar READY 后崩溃→限次重启** 无自动化 | 高 | `main.ts:757-809` + `watchSidecarExit` 无测试文件覆盖；耗尽弹框路径未测 |
| G4 | **main 单飞/互斥挂接** 无集成测 | 中 | `openRemoteStartFlight`/`CompleteFlight`/`sidecarLifecycle` 仅声明与调用；双击 Connect / 切工作区孤儿场景靠手测 |
| G5 | **IPC / before-quit 接线** 未测 | 中 | pause/stop/retry IPC（`main.ts:1538-1551`）与 `before-quit`→`abortAll`（`:1850-1858`）无测；Manager 层已绿 |
| G6 | **Hub/web 未接 pause/stop UI** | 低（已知，禁区不碰 web） | WP-035 已记；preload API 有，产品 UI 无按钮 |
| G7 | **无历史 target 时只断开不重连** | 低（已知行为） | `onTunnelUnexpectedExit` 在 `!targetId \|\| !workspace` 直接 return |
| G8 | **多实例并发下载（D11）** | 低（未做） | WP-035 已知 |
| G9 | **Electron 手测 5 项未跑** | 中 | WP-035 手测清单全未勾；本步只读不补手测环境 |

### 统计

| 维度 | 已覆盖 | 部分覆盖 | 缺口（能力行 / 缺口表） |
|---|---|---|---|
| A WP-035/041（9） | 5（A4–A8） | 4（A1–A3, A9） | — |
| B 既有壳（9） | 9 | 0 | — |
| C scripts/WF | 真断言 4 项；仅日志 1；CI 单测假绿风险 1 | — | G1 |
| 缺口表条目 | — | — | **9**（G1–G9） |
| **npm test** | **127 pass / 0 fail** | | |

**合计能力行（A+B=18）**：已覆盖 **14** / 部分 **4** / 能力级缺口 **0**（编排类记在「部分」+ G2–G5）。

### 给前端中层（步 1 过眼）

步(1) 覆盖绿表+缺口已写入 `WP-044-desktop.md`。`cd desktop && npm test` → **127/0**。WP-041 下载契约（pause/stop/retry/Range/进度/清扫）与 D8 辅助、既有壳不变量均有单测；最大洞是 **CI 不跑 desktop 测**、以及 **main 里隧道/sidecar 自动恢复与单飞挂接无集成测**。请过眼后我将开步(2)只列裁剪提案、裁决前不删。


---

## 裁剪提案

> WP-046 步(2) · 2026-10-12 20:20 HKT · **裁决前不执行**；不删不改任何测试/产品代码。  
> 对照：`WP-021-frontend.md` Part 2（18 条）+ web 互审异议；复核 WP-041 新增测。  
> 基线仍为步(1)：`npm test` → **127 pass / 0 fail**（显式登记 21 个 `dist/*.test.js`）。

| id | 来源 | 测名或路径 | 类别 | 理由 | 提案（删/留/放宽怎么改） | 旧清单对照 |
|---|---|---|---|---|---|---|
| C01 | WP-021 P2 | `sidecar.test` › stopSidecar「is a no-op for a null handle」 | 可删→**留** | 仍是 `sidecar.ts:215` 一句 `if (!handle) return`；web 互审要求保留（公共 API 契约） | **留** | 仍成立（事实琐碎）；**提案过时**（原「可删」作废） |
| C02 | WP-021 P2 | `ssh-credential-material.test` › removeMaterializedKey「is a no-op for an empty key directory」 | 可删→**留** | 仍琐碎；真正清理已由 `withMaterializedKeyDirectory` 覆盖；web 互审要求保留 | **留** | 仍成立；**提案过时** |
| C03 | WP-021 P2 | `slim-packaging.test` ›「package_win.ps1 defaults to slim…」 | 可放宽 | 仍 `assert.match(/Default = slim/)` + leak-throw 源码针；改注释可红、逻辑坏可能仍绿；真门在 `package_win.ps1` 运行时 throw | **放宽**：去掉「Default = slim」文案针，只留 leak-throw 存在性；或改 CI 解包断言（见 CI 建议） | **仍成立** |
| C04 | WP-021 P2 | `slim-packaging.test` ›「assemble_product.ps1 only copies models when -WithModel」 | 可放宽 | 仍锁 `$IncludeModel = [bool]$WithModel` 等源码标识 | **放宽**：只断言「无 -WithModel 时不出现无条件 Copy models」一类结构针，或并入 C03 | **仍成立** |
| C05 | WP-021 P2 | `bundle-paths.test` ›「candidate list order matches packaged→…→development」 | 可放宽 | 行为已由「earlier linux candidates win」覆盖；本用例锁内部候选数组 | **放宽/删**：优先删本条，保留成对赢用例 | **仍成立** |
| C06 | WP-021 P2 | `sidecar.test` › formatSidecarBootFailure ×2 | 可放宽 | 仍偏字符串快照 | **放宽**：只断言含 code/bin/workspace/log 关键字段，不锁整句 | **仍成立** |
| C07 | WP-021 P2 | `bundle-paths.test` › 单源优先（MODEL_DIR / LINUX_BUNDLE / BUNDLE_ROOT，约 L84/91/99） | 重复 | 全序「locks the full precedence」+ pairwise 已覆盖同优先级 | **删**三条单源用例（保留全序+pairwise+dev-only packaged 门） | **仍成立** |
| C08 | WP-021 P2 | `remote-progress.test` › remoteDownloadRatio ×2 + mapComponentProgressToRemote ×2 | 重复→**可放宽** | web 互审：**不算重复**——边界（total=0/undefined、overshoot）ensure 端到端未覆盖 | **放宽**：合并为表驱动，**不删边界行** | **过时**（原「重复/可删」定性作废） |
| C09 | WP-021 P2 | `ssh-credential-material.test` ›「recursively removes the temp directory」 | 重复 | 与 `withMaterializedKeyDirectory` 正常/抛错两条重叠 | **删**本条；保留 withMaterialized* 两条 | **仍成立** |
| C10 | WP-021 P2 | `remote-progress.test` 整体相对 main 接线 | 假绿→**接线未覆盖** | `:107` 退订是真门；缺口只在 `main` subscribe 接线（步1 G5） | **留**全部；勿标整文件假绿 | **过时**（定性） |
| C11 | WP-021 P2 | `desktop/package.json` `test` 显式枚举 `dist/*.test.js` | 假绿风险 | 现 **21/21** 已登记（WP-041 增 3）；忘登记仍静默不跑 | **放宽实现**：改 glob 或加「src *.test.ts 与 scripts.test 列表一致」检查测 | **仍成立（加重）** |
| C12 | WP-021 P2 | `scripts/smoke_product.sh` vs `.ps1` | 重复 | 同套 READY/401/锁/重启；CI 只跑 `.ps1`（`windows-sidecar.yml`） | **留** `.ps1` 为 CI 真源；`.sh` 标「本地/非 Windows」或文档说明，**不删**（跨平台手跑） | **仍成立** |
| C13 | WP-021 P2 | `smoke_product.ps1:116-118` 端口复用比较 | 假绿 | 两分支皆 `Write-Host`，无 assert | **删**两行日志，或改成注释说明「端口可同可异均 ok」 | **仍成立** |
| C14 | WP-021 P2 | `windows-sidecar.yml` Cache embed models + models 存在性 echo | 可删 | 仍 `-SkipModel`；`:60-62` 仅日志；cache 对 smoke 无断言 | **删** echo；cache 步可删或保留（见 C16） | **仍成立** |
| C15 | WP-021 P2 | `windows-desktop.yml` CSC 检测 echo（约 45–50） | 可删 | 两分支只打印，不影响 `package_win.ps1` | **删** if/else echo，直接跑打包 | **仍成立** |
| C16 | WP-021 P2 | desktop / signed-release「Cache embed models」 | 可删 | `git ls-files models/**` 仍可见 onnx/.data；cache 冗余（若变 LFS 指针再开） | **删** cache 步（假设已证实入 git） | **仍成立** |
| C17 | WP-021 P2 | `windows-signed-release.yml` `web/node_modules` 存在分支 | 可删 | 全新 runner 几乎总走 else `npm ci` | **删** if，直接 `cd web && npm ci` | **仍成立** |
| C18 | WP-021 P2 | `windows-desktop` / signed-release 作「测试门」 | 假绿 | 仍不跑 `desktop npm test`（步1 G1） | **不删 workflow**；门的定义见下节 CI 建议 | **仍成立** |
| C19 | WP-041 新 | `components.test` ›「tracks sliding-window speed and percent」（SpeedTracker）vs「emits status, percent, and bytesPerSecond」 | 重复/可放宽 | 纯函数窗速与 Manager 发射有重叠；后者已集成覆盖 percent/Bps | **放宽**：SpeedTracker 可并入 emit 用例作前置，或只留滑动窗边界（若 emit 不锁窗算法则 **留** SpeedTracker） | **新** |
| C20 | WP-041 新 | `process-recovery`「never recovers intentional」vs `sidecar`「marks intentionalStop before killing」 | —（易误判重复） | 一测策略、一测 stop 打标；合删任一则失唯一覆盖 | **留两者**；禁止「合并删一」 | **新**（明确不裁） |
| C21 | WP-041 新 | `abortAll` 测 vs `pause` 测 | — | 行为像 pause，但入口是 quit 路径唯一自动化 | **留** abortAll | **新**（明确不裁） |
| C22 | WP-041 新 | `lifecycle-mutex` / `remote-state` D8 / SSH tunnel args / pause·Range 族 | — | 步1 评为已覆盖或唯一辅助门；无旧测等价物 | **全部留**；不得因「辅助测」删掉某不变量唯一覆盖 | **新**（明确不裁） |
| C23 | 安装残留断言 | `ComponentManager`「installs both…」末尾「temp files cleaned up」vs `cleanupComponentTmp` 启动清扫测 | 弱相关 | 前者=成功路径 finally；后者=启动清陈旧 `.part`；不变量不同 | **留两者** | **新**（明确不裁） |

**辩证摘要（裁决前不执行）**

- 过时可动：C03–C07、C09、C13–C17 的删/放宽方向仍合理。  
- **勿删唯一覆盖**：C01/C02（互审后留）、C08 边界、C10 退订、C20–C23、pause/Range/D8/mutex/tunnel-args。  
- 假绿真凶仍是 **C11 + C18**（列表漏登记 + CI 不跑测），不是 remote-progress 纯函数本身。

### 统计（裁剪提案）

| 类别（提案口径） | 条数 | id |
|---|---|---|
| 可删（仍建议删） | 7 | C07, C09, C13, C14†, C15, C16, C17 |
| 可放宽 | 6 | C03, C04, C05, C06, C08, C19‡ |
| 重复（已计入可删/放宽） | — | C07/C09；C19 弱重复 |
| 假绿 / 假绿风险 | 3 | C11, C13, C18（C10 不再算） |
| 留 / 明确不裁 | 8 | C01, C02, C10, C12§, C20, C21, C22, C23 |
| **提案表总行** | **23** | C01–C23 |

† C14 echo 可删；cache 与 C16 合并裁决。  
‡ C19 视 emit 是否覆盖窗算法二选一。C05 可放宽或删，计入可放宽。  
§ C12 留双脚本、标职责，不算「可删」。

| 相对 WP-021 旧 18 条 | 条数 |
|---|---|
| **仍成立**（分类/方向大体可沿用） | **14**（C03–C07, C09, C11–C18） |
| **过时**（定性或提案被互审/现状推翻） | **4**（C01, C02, C08, C10） |
| **新增**（WP-041 后） | **5**（C19–C23） |

---

## CI 建议

> **只建议，不实施**（本包禁区：桌面 CI / 不补集成测）。

| # | 建议 | 动机 | 不做的理由（本包） |
|---|---|---|---|
| CI-1 | 在 `windows-desktop.yml`（或独立 `desktop-test.yml`，paths: `desktop/**`）于 package 前加 `cd desktop && npm ci && npm test` | 关闭 C18/G1：ISSUE-028 + WP-041 门进 CI | 父包禁「桌面 CI」擅自做 |
| CI-2 | `package.json` `test` 改为 `node --test dist/**/*.test.js` 或加一致性测（对 C11） | 防漏登记假绿 | 属测试脚手架改动，需裁决后另包 |
| CI-3 | 可选：CI 解包 slim NSIS 断言无 `resources\sidecar\models` / `linux`（强化 C03） | 源码 regex 放宽后的运行时门 | 需 Windows runner 产物；另包 |
| CI-4 | `windows-signed-release` 加 `concurrency`（旧 D10） | 防 tag+release 双跑 | 非本步范围；仅记录 |
| CI-5 | 不在此包补 main 隧道/sidecar 集成测 | 步1 G2/G3 仍缺口 | 明确「不补集成测」 |

### 给前端中层（步 2）

WP-046 步(2) 裁剪提案已写入 `WP-044-desktop.md`（23 行；裁决前不执行）。相对 WP-021：14 仍成立 / 4 过时 / 5 新增。建议优先放行的删/放宽：**C07 三条单源 bundle-paths**、**C09 removeMaterializedKey 递归**、**C13 smoke 端口日志**；CI 仅建议加 `npm test`（不实施）。请裁决后我再执行删改（另包或续批）。

## 裁决与落地

- 包：WP-046（owner frontend_desktop，父包 WP-044）
- 时间：2026-10-12 20:25 HKT（Asia/Hong_Kong）
- 依据：前端中层裁决（删 C07/C09/C13–C17/C05；放宽 C03/C04/C06/C08/C11；留 C01/C02/C10/C12/C18–C23）
- 验证：`cd desktop && npm test` → **127 pass / 0 fail**（未 commit；未 deliver；未接桌面 CI；未补 G2–G5）

| id | 动作 | 文件变更 | 说明 |
|---|---|---|---|
| C07 | 删 | `desktop/src/bundle-paths.test.ts` | 删三条单源：`resolves LITECODE_MODEL_DIR over sidecar models`、`resolves linux tar from LITECODE_LINUX_BUNDLE`、`resolves linux tar from LITECODE_BUNDLE_ROOT/linux`；保留全序+pairwise+dev-only |
| C05 | 删 | `desktop/src/bundle-paths.test.ts` | 删`candidate list order matches packaged→…→development`；保留`earlier linux candidates win…` |
| C09 | 删 | `desktop/src/ssh-credential-material.test.ts` | 删`recursively removes the temp directory`；保留 withMaterialized* 与 no-op |
| C13 | 删 | `scripts/smoke_product.ps1` | 端口复用两分支 Write-Host 改为注释（不断言） |
| C14 | 删 | `.github/workflows/windows-sidecar.yml` | 删 Cache embed models 步 + models 存在性 echo |
| C15 | 删 | `.github/workflows/windows-desktop.yml` | 删 CSC 检测 if/else echo；直接 `./scripts/package_win.ps1` |
| C16 | 删 | `windows-desktop.yml`、`windows-signed-release.yml` | 删两处 Cache embed models（sidecar 已在 C14 删） |
| C17 | 删 | `.github/workflows/windows-signed-release.yml` | `web/node_modules` if 改为直接 `(cd web && npm ci)` |
| C03 | 放宽 | `desktop/src/slim-packaging.test.ts` | 去掉 `Default = slim` 文案针；保留 leak-throw / Full switch / filter 改写 |
| C04 | 放宽 | `desktop/src/slim-packaging.test.ts` | 改为结构针：`$IncludeModel` + gated `Copy-Item models\*`；去掉 Default/文案针 |
| C06 | 放宽 | `desktop/src/sidecar.test.ts` | formatSidecarBootFailure 只断言 code/bin/log；空输出行为保留 |
| C08 | 放宽 | `desktop/src/remote-progress.test.ts` | remoteDownloadRatio 边界合并为表驱动 7 行；不丢 total=0/undefined/overshoot |
| C11 | 放宽 | `desktop/package.json` | `test` 改为 `node --test dist/*.test.js`（tsc + copy-theme-tokens 后；壳扩展 glob） |
| C01/C02/C10/C12/C18–C23 | 留 | — | 未改（含 SpeedTracker C19） |

### 绿表（落地后）

`cd desktop && npm test` → tests **127** / pass **127** / fail **0**（删 5 条用例、表驱动净增 5 条边界行，计数持平）。
