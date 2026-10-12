# WP-018 前端：揪不稳实现 + 裁冗余测试（只列不删）

- 范围：`web/src`、`desktop/src`、`scripts/`、`.github/workflows/windows-*.yml`；分支 `ops/team-framework`
- 约束：未改产品代码、未改/删测试、未 commit；未触 WP-016 引擎 UI、WP-004 symlink 复制
- 基线：`cd web && npx vitest run src/api/` → **68 pass / 0 fail**（14:19 HKT，含 8 个 death-list 文件）

---

## 一、可疑不稳实现（11 条）

| # | 类别 | 证据 | 问题 | 风险 | 建议修法 |
|---|---|---|---|---|---|
| F1 | 重连/RPC | `web/src/api/agentWs.ts:95-105`（connect 换代）、`:165-188`（onclose）、`:77`（pendingRpc） | 换代/断线时 **不 reject `pendingRpc`**；旧 socket 上的请求响应永远不会到新 socket，调用方只能等 30s 超时（`:282-285`） | 断线后 set-model/revert/permission 等 RPC 静默挂 30s；`replayFromAnchor` 串行 await 会整体卡死后才回滚 | `onclose`/`connect()` 换代时遍历 `pendingRpc` 全部 `reject(new Error("WebSocket closed"))` 并清空；补单测 |
| F2 | 重连退避 | `agentWs.ts:133-137` | `onopen` 即把 `reconnectAttempt=0` 并置 `connected`，**早于 server/hello** | 服务端接受 upgrade 后在 hello 前断开（崩溃循环/代理）时，退避每次被清零 → 以 base 延迟紧循环重连；UI 短暂显示已连接 | 把 `reconnectAttempt=0` 与 `connected` 状态移到收到 `server/hello`（`:415-418`） |
| F3 | RPC 发送 | `agentWs.ts:304` vs `:262-268` | `sendJsonRpc` 的 `ws.send` 无 try/catch（`send` 有，FE-10）；抛错时 promise 被 executor 拒绝，但 `pendingRpc` 条目与 30s 定时器泄漏 | 小泄漏+错误口径不一致 | 同 `send` 包 try/catch，失败时 `pendingRpc.delete`+`clearTimeout` |
| F4 | Ask 多会话/多请求 | `web/src/stores/turnStore.ts:59`（`pendingPermission` 单槽）、`:686-711` | 每 session 仅一个待答槽，后到的 `permission_request`（并行工具调用/子 agent）**直接覆盖**前一个；前一个 `request_id` 永不应答 | 服务端回合挂在第一个 ask 上，用户只能看到第二个；表现为"卡住" | 改为按 `request_id` 的队列（FIFO），grant 后出队；或确认服务端保证单 ask 串行并加断言注释 |
| F5 | Ask 重连恢复 | `web/src/stores/sessionStore.ts:176-185`（snapshot 只回灌 turn/llm_reconnect/meter/bash） | snapshot 不含待答 ask；新窗口/刷新/重新 attach 后 ask 卡片不再出现 | 回合永久阻塞直到取消 | 服务端 snapshot 带 `pending_permission`，前端 `applySnapshot` 回灌（需协议域配合） |
| F6 | Ask 重复提交 | `turnStore.ts:644-668` | 卡片在回执前保持可点（设计如此，FE-04），但 **无 in-flight 锁**；双击发两次 `agent/permission` | 第二次可能被服务端判未知 request → toast 报错；或 always 规则重复写 | slice 加 `permissionInFlight: request_id`，同 id 未回执前忽略 |
| F7 | 取消失败静默 | `turnStore.ts:618-630` | `cancel` 乐观置 `cancelling` 并清 ask，RPC `.catch(() => {})` 静默 | RPC 失败时 UI 永远 cancelling，与 FE-04「禁静默 catch」原则相悖 | catch 中回滚 `runState:"running"` + toast |
| F8 | LLM 重连重试 | `turnStore.ts:997-1009` | `retryLlmReconnect` 无 in-flight 锁；状态要等服务端事件才变化，快速双击发两次 `agent/retry` | 可能重复唤醒同一 transcript | 发送前本地置 `llmReconnect.phase="retrying"`（或 inflight flag），失败回滚 |
| F9 | 下载失败路径 | `desktop/src/components.ts:405-418`（downloadOnce）、`:383-403` | 空闲定时器只在拿到 `res.body` 后才 arm；`fetchImpl(url)` **到响应头前无超时** | 网络黑洞/代理挂起时 Open Remote 进度永久停在 "Downloading 0 MB"，也不进入重试 | fetch 前就 `arm`（或 `AbortSignal.timeout(CONNECT_TIMEOUT)`），头到后切换为 idle 定时 |
| F10 | 瘦包安装竞态 | `components.ts:736-745`（installModel）、`:751-760`（installLinuxTar）；注释 `:739` 自认"Another window may have finished first" | 多进程共享 bundles 目录时：model 的 exists→rename 非原子，第二个进程 `renameSync(staging, finalDir)` 会 ENOTEMPTY/EPERM 报错而非复用就绪安装；linux tar 在 `rmSync(checksum)` 与写回之间对外可见"无 checksum 的 tar"，Windows 上 rename 覆盖被占用文件会 EPERM | 并发 Open Remote 偶发 "download failed"；另一进程误判缺失再次下载 | rename 失败时 re-check `isModelDirReady(finalDir)` 则返回；tar 先写 `checksum.tmp` 再双 rename，或按进程锁文件串行；启动时清理 `bundles/.tmp` 孤儿 `.part` |
| F11 | Sidecar 生命周期竞态 | `desktop/src/main.ts:1051-1070`（relaunchSidecar）、`sidecar.ts:176-182`（exit 只在 READY 前监听） | ① `relaunchSidecar` 无串行锁，两次并发打开工作区 → 两个 `startSidecar`，前一个 handle 被覆盖成孤儿进程；② READY 后 sidecar 崩溃无 exit 监听，主进程不知情，只靠 web 端 WS 无限重连 | 孤儿进程占端口/锁库；崩溃后 UI 一直 reconnecting 无可操作提示 | 用 promise 链串行 relaunch；READY 后挂 `child.once("exit")` 通知渲染层并提供重启 |

附（CI/脚本，非阻断）：
- C1 `.github/workflows/windows-desktop.yml:38-58`：打包前 **不跑 web vitest / desktop node:test**；且 `build_components.mjs --allow-missing` 可产出缺组件的 manifest → 运行期才在 F9/F10 路径暴露。建议加测试步骤，release workflow 去掉 `--allow-missing`（nightly 可保留）。
- `scripts/*.ps1` 均 `ErrorActionPreference=Stop`，`assemble_product.ps1:68-75` 临时 Continue 但显式检查 `$LASTEXITCODE`，**无问题**。

---

## 二、WP-017 R2：web death-list basename 自指弱断言

来源：`.litecode/ops/artifacts/WP-017.md:186,251,257`（承 WP-012 `:36,61,66`）。

### 2.1 定位（5 处自指断言）

| 文件:行 | 断言 |
|---|---|
| `web/src/api/r5-death-list.test.ts:44-46` | `expect(basename(__filename)).toBe("r5-death-list.test.ts")` |
| `web/src/api/r6-death-list.test.ts:53-55` | 同上（r6） |
| `web/src/api/r9-death-list.test.ts:70-72` | 同上（r9） |
| `web/src/api/session-seq-a-death-list.test.ts:100-102`（文件内 53-55） | 同上（session-seq-a） |
| `web/src/api/session-seq-d-death-list.test.ts:42-44` | 同上（session-seq-d） |

（r4 / serve-auth / model-selection 无此断言。）

### 2.2 为何弱
1. **恒真/同义反复**：断言自身文件名等于字面量，与任何产品代码无关；唯一能让它失败的是重命名文件——而重命名时人会顺手改字面量。它不守任何不变量，只贡献 5 个"绿"计数（假绿膨胀）。
2. **它想守的东西没守住**：真正的意图是"排除规则不会把 gate 自身/产品文件漏扫"，但：
   - 排除用 `name.includes("death-list")`（r5:21、r6:30、r9:33、model-selection:24、seq-a/seq-d）——任何**产品**文件名含 `death-list` 都会被静默豁免；
   - 没有断言 `files.length > 0`：若 `WEB_SRC`（`join(__dirname,"..")`）因目录迁移指错，walk 结果为空，violations=[] **全部假绿**；
   - 排除口径不一致：r5/r6/r9/model-selection 只排 death-list，**扫其它 *.test.ts**；r4/serve-auth 排所有 test；seq-a/seq-d 两者都排。同一针在不同 gate 中对测试文件的约束不同。
3. 当前 `rg` 确认非 death-list 文件中无 `partialText|tool_start|liveTools|revert_messages|HookMessage`，故现状绿是真绿，但 gate 本身对上述回归无感。

### 2.3 加强方案（补丁草案，未落盘到测试）
抽共享 helper `web/src/test/deathListGate.ts`，各 gate 删除 basename 用例，改为两条有意义断言：

```ts
// web/src/test/deathListGate.ts（草案）
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
export const WEB_SRC = join(__dirname, "..");
const GATE_RE = /^[\w-]+-death-list\.test\.ts$/;          // 只豁免 gate 本身，精确匹配
export function collect(opts: { includeTests: boolean }): string[] {
  const out: string[] = [];
  const walk = (d: string) => { for (const n of readdirSync(d)) {
    const p = join(d, n); if (statSync(p).isDirectory()) { walk(p); continue; }
    if (!/\.(ts|tsx)$/.test(n) || GATE_RE.test(n)) continue;
    if (!opts.includeTests && /\.test\.tsx?$/.test(n)) continue;
    out.push(p); } };
  walk(WEB_SRC); return out;
}
export function scan(files: string[], needles: readonly string[]): string[] {
  const v: string[] = [];
  for (const f of files) { const s = readFileSync(f, "utf8");
    for (const n of needles) if (s.includes(n)) v.push(`${relative(WEB_SRC, f)}: \`${n}\``); }
  return v;
}
```
```ts
// 每个 gate 中替换 basename 用例：
it("gate actually scans production sources (non-vacuous)", () => {
  const files = collect({ includeTests: false });
  expect(files.length).toBeGreaterThan(50);                       // 防 WEB_SRC 漂移致空扫
  expect(files.some(f => f.endsWith(`stores${sep}turnStore.ts`))).toBe(true); // 锚点：关键产品文件在扫描集内
});
it("scan() flags a planted needle (matcher self-test)", () => {
  const tmp = join(tmpdir(), `planted-${Date.now()}.ts`);
  writeFileSync(tmp, `export const x = "${FORBIDDEN[0]}";`);
  try { expect(scan([tmp], FORBIDDEN).length).toBe(1); } finally { rmSync(tmp); }
});
```
- 统一口径：**产品 gate 一律 `includeTests:false`**（测试文件可合法出现禁词作反例），消除 r5/r6/r9 与 r4 的不一致。
- 豁免改精确正则 `GATE_RE`，产品文件即便名含 death-list 也会被扫。
- 结论：R2 建议「删 5 条 basename 用例 + 加 non-vacuous 锚点断言 + 抽 helper」，不进 slim CI 必过集（维持 WP-012/017 结论）。需 GM 批准后由执行者落地。

---

## 三、前端测试「可删/可放宽/重复/假绿」清单（10 条，只列不删）

| # | 分类 | 测试 | 证据 | 理由 | 建议 |
|---|---|---|---|---|---|
| T1 | 可删（假绿） | 5 条 `gate file itself is named for …`（见 2.1） | 见 2.1 | 同义反复，不守任何不变量 | 删，替换为 non-vacuous 锚点（R2 方案） |
| T2 | 重复 | R9 `FORBIDDEN_PRODUCTION` 含 `tool_start/tool_end/liveTools`（`r9-death-list.test.ts:13-18`）与 R4（`r4-death-list.test.ts` FORBIDDEN）重叠；`partialText/partialReasoning` 与 R5（`r5:11`）完全重叠 | 文件头 r9:3-4 自述"R3–R6 continuity" | 同针双门，回归时双红噪声；R5 整体被 R9 覆盖 | 删 R5 文件，或从 R9 移除 R4/R5 已覆盖针；二选一保留单一真源 |
| T3 | 可放宽/合并 | 8 个 death-list 各自复制 `walkTsFiles`（r4:25、r5:13、r6:24、r9:27、seq-a:24、seq-d:13、serve-auth:23、model-selection:18） | 排除规则三种口径（见 2.2） | 重复实现导致口径漂移 | 抽 `deathListGate.ts` 共享 helper |
| T4 | 弱断言 | `r9-death-list.test.ts:55-68`「accepts shared golden Item JSON fixtures」 | 只断言 `item.type ∈ {message,function_call,reasoning}` | 不调用 FE adapter，未证 FE 能解析 Rust serde 金样；名不副实 | 放宽为删除，或加强为把 fixture 喂给 `api/adapter.ts` 并断言投影结果 |
| T5 | 弱断言 | `model-selection-death-list.test.ts:54` `src.includes("modelId")` 为 true | ModelSwitcher 必然出现该标识符 | 正向子串断言几乎恒真 | 删此行；`"Missing: "` 断言（:56）保留或改为渲染测试 |
| T6 | 过时（死亡名单辩证） | `serve-auth-death-list.test.ts` 中 `"Auth token (dev only)"`、`"Save auth"` 等 UI 文案针 | `serve-auth-death-list.test.ts:18-19` | 守的是已删 UI 文案，任何改写措辞即可绕过；真正不变量是 API 路由与类型名（`/api/settings/auth`、`AuthWriteResponse`），已在同表 | 放宽：去掉文案针，保留路由/类型针 |
| T7 | 保留（辩证结论） | `session-seq-a/d-death-list.test.ts` 主扫描用例、r4、r6 | WP-017:183「仍有效」；本次 68/68 绿 | 守 seq 身份/持久化同构，Rust 侧 `death_list_gate` 有对应 | 保留，仅删 basename 用例 |
| T8 | 脆弱（假红风险） | `desktop/src/slim-packaging.test.ts:38,49,51` 匹配脚本注释/文案（`/Default = slim/`、`/Default is slim/`、`slim product: embed model not bundled`） | 同文件 | 改注释即红、改逻辑不红；与 WP-012「轻量打包卫生门」意图不符 | 放宽：去掉注释类 regex，保留 `!models/**` filter 与 `throw … leak` 结构断言；真门应在 CI 解包后检查（C1） |
| T9 | 已解决（勿再列） | `debug-fold.test.tsx` 恒真（WP-012:36） | 文件已不存在；`git log` 最近改动于 `c6b24df` | 已处理 | 从待办移除 |
| T10 | 已解决（勿再列） | ISSUE-008 PermissionModal `justify-start` 假红 | `web/src/components/PermissionModal.test.tsx:260-261` 已改为注释说明，不再断言 | 已处理；本次 api 目录 68 绿 | 可关闭 ISSUE-008 关联项（需 WP-005 确认） |

覆盖缺口（非裁剪，供后续补测）：`agentWs` 无「断线 reject pendingRpc」「hello 前断开退避不清零」用例（F1/F2）；`turnStore` 无「两个 permission_request 连发」「cancel RPC 失败回滚」「retry 双击」用例（F4/F7/F8）；`components.test.ts` 无「响应头前挂起」「并发 install rename 冲突」用例（F9/F10）。

---

## 统计
- 一、不稳实现：11 条（F1–F11）+ 1 条 CI（C1）
- 二、R2：5 处定位 + 3 点弱因 + 1 份补丁草案
- 三、测试清单：10 条（可删 1 / 重复 2 / 弱断言 2 / 过时放宽 1 / 脆弱 1 / 保留 1 / 已解决 2）
