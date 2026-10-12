# WP-044-web（WP-045）— web 核心覆盖核对 + 测试裁剪提案

父包：WP-044（frontend_mid） / 祖包：WP-042  
执行：frontend（前端-web）  
范围：仅 `web/`。不 commit。未碰 desktop / 后端。未删测试。

---

## 覆盖

> Step 1（2026-10-12 20:15 HKT）。方法：`rg` + 打开测试读断言（不只看文件名）；只读 vitest（`/exec-daemon/node` v22.14.0）跑相关 9 文件。  
> 结论口径：**绿** = 有断言直接守该不变量；**弱** = 实现在、相关测在，但未正面断言该点或只测单元素/隔离组件；**缺** = 无有效回归用例。

### 绿表

| # | 能力 | 测证据（文件:行 / 用例名） | 结论 |
|---|------|---------------------------|------|
| C1 | `permissionQueue` FIFO：并行入队、grant 只出队头 | `turnStore.test.ts:1033` `FIFO: parallel asks enqueue; grant dequeues head only` — 断言 `["req-a","req-b"]` → 回执后 `["req-b"]` | 绿 |
| C2 | 同 `request_id` 不双入队（原地更新） | `turnStore.test.ts:1164` `duplicate request_id updates in place instead of double-enqueue` — length 1、summary 更新 | 绿 |
| C3 | grant 成功出队 | `turnStore.test.ts:825` `keeps the card open until the agent/permission receipt arrives, then closes` | 绿 |
| C4 | grant 失败且未 dismiss：队头保留 + toast（非「复活」路径） | `turnStore.test.ts:847` `rolls the card back and surfaces an explicit error when the receipt fails` | 绿 |
| C5 | grant 失败不复活已 dismiss 卡（cancel 清队后） | `turnStore.test.ts:1074` `W6: cancel mid-grant does not resurrect…` — cancel 后 length 0，reject 后仍 0 | 绿 |
| C6 | grant 失败不复活已 dismiss 卡（transcript revert 清队后） | `turnStore.test.ts:1119` `W6: transcript revert mid-grant does not resurrect…` | 绿 |
| C7 | cancel 清队 | 同 C5：`cancel` 后 `permissionQueue` length 0（`:1105-1108`） | 绿 |
| C8 | transcript revert 清队 | 同 C6：`onTranscriptReverted` 后 length 0（`:1150-1153`） | 绿 |
| C9 | PermissionCard `key=request_id` remount 清空旧勾选 | `PermissionModal.test.tsx:512-544` `key remount clears prior ask picks` — key 从 req-1→req-2 后 Submit 再 disabled | 绿 |
| C10 | agentWs：socket close 立刻 reject pendingRpc（不干等 30s） | `agentWs.test.ts:277` `rejects in-flight RPC immediately on socket close` | 绿 |
| C11 | agentWs：disconnect 立刻 reject | `agentWs.test.ts:289` `rejects in-flight RPC on disconnect` | 绿 |
| C12 | agentWs：connect 换代立刻 reject | `agentWs.test.ts:299` `rejects in-flight RPC when connect() bumps generation` | 绿 |
| C13 | close 后推进 31s 不二次 timeout reject | `agentWs.test.ts:310` `does not fire the 30s timeout after close already rejected` | 绿 |
| C14 | PermissionModal 三态 Ask（permission / approval·plan / ask_user）交互 | `PermissionModal.test.tsx` plan/tool/ask_user 共 17 用例（含 answers map、Cancel、多题） | 绿 |
| C15 | ComposerDock 叠卡 + Allow → grantPermission | `ComposerDock.test.tsx:51-82` overlay 非 fullscreen；Allow once 调 `grantPermission` | 绿 |
| C16 | composer 发送与回合态（queue / start / reject 回滚） | `AgentChatInput.test.tsx:85+` silent send / queue；`turnStore.test.ts:716-779` `start`→`agent/run`、reject 回 idle | 绿 |
| C17 | 消息 snapshot / delta（buffer/item、gap catch-up、revert） | `messageStore.test.ts:192` buffer/item replaces；`:376` reverted；`:583-650` applySnapshot retained window catch-up | 绿 |
| C18 | 重连表面（WS liveness + LLM reconnect UI/文案） | `agentWs.test.ts:160+` liveness/reconnect；`LlmReconnectBubble.test.tsx`；`turnStore.test.ts:1465` reconnect copy | 绿 |
| C19 | 子 agent 转发的 permission 事件不进父会话 | `connectionStore.test.ts:13` `ignores … permission when parent_session_id is set` | 绿 |
| C20 | UI 只渲 `permissionQueue[0]` + `key={request_id}` 接线 | 实现：`AgentPanel.tsx:552` / `:615`。测：ComposerDock 只喂单元素队列；AgentPanel.test mock 掉 PermissionCard；**无**「队列长度≥2 时 DOM 仅一张卡」断言；key 行为靠 C9 组件级 | 弱 |
| C21 | 断线后不自动重放已 reject 的 RPC | 实现：`agentWs.ts:275` 注释 + `rejectAllPendingRpc` 只 reject/clear；connectionStore 无重放队列。测：C10–C13 证 reject，**无**「重连后不 resend 同 id」正面断言 | 弱 |
| C22 | 孤儿 ask 不恢复（snapshot 不回灌 pending permission） | 协议：`SessionSnapshot`（`types.ts:441+`）无 `pending_permission`；`sessionStore.applySnapshot` 只调 turn/meter/bash，不写 `permissionQueue`。**缺**「applySnapshot 后 queue 仍空」回归用例 | 弱 |
| C23 | `onTurnStarted` 清 `permissionQueue` | 实现：`turnStore.ts:751`。测试中多处调 `onTurnStarted`，**无一**断言清队 | 缺 |
| C24 | `onTurnFinished` 清 `permissionQueue` | 实现：`turnStore.ts:979`。**无**清队断言 | 缺 |
| C25 | 多 session Ask 隔离（A 的 ask 不影响 B 的批复/展示） | store 按 `byId` 分片，天然隔离；**无**双 session 各持 ask、grant A 不动 B 的用例。近邻：`AgentChatInput` recall 跨 session 不串、notificationStore session isolation — 非 Ask | 缺 |

### 缺口列表（建议补测方向；本 Step 不写新测）

1. **C23/C24**：seed 非空 `permissionQueue` → `onTurnStarted` / `onTurnFinished` → 断言 `[]`。
2. **C20**：ComposerDock/AgentPanel 喂双元素队列，断言 DOM 仅一张 PermissionCard，且 `key` 随队头 `request_id` 变。
3. **C21**：close reject 后 `connect`+hello，断言 `send` 帧中无自动重发的旧 rpc id（或 spy `sendJsonRpc` 无重放）。
4. **C22**：`applySnapshot`（含刻意多字段的 fixture）后该 session `permissionQueue` 仍为 `[]`；可选断言类型/adapter 不识别 `pending_permission`。
5. **C25**：session A/B 各入队 ask → grant A → A 出队、B 不变；或挂载两 ComposerDock 断言各渲各的队头。

### 只读 vitest 基线

```
/exec-daemon/node node_modules/vitest/vitest.mjs run \
  src/api/agentWs.test.ts \
  src/stores/turnStore.test.ts \
  src/components/PermissionModal.test.tsx \
  src/dockview/panels/ComposerDock.test.tsx \
  src/dockview/panels/AgentPanel.test.tsx \
  src/components/AgentChatInput.test.tsx \
  src/stores/messageStore.test.ts \
  src/stores/connectionStore.test.ts \
  src/components/LlmReconnectBubble.test.tsx
```

结果：**9 files / 165 tests passed**（系统 Node 20 无法跑 vitest5/jsdom/undici；用 exec-daemon Node 22.14.0）。

### Step 1 小结

| 结论 | 条数 |
|------|------|
| 绿 | 19（C1–C19） |
| 弱 | 3（C20–C22） |
| 缺 | 3（C23–C25） |

WP-034 主行为（FIFO / W6 不复活 / cancel·revert 清队 / rejectAllPendingRpc 三路径 / key remount）均有绿证据；弱/缺集中在 UI 双队头、无重放正面断言、snapshot 孤儿回归、turn start/finish 清队、多 session Ask。

---

## 裁剪提案

> Step 2（2026-10-12 20:20 HKT）。只提案、**裁决前不删**。本包不补新测（Step1 缺口 C20–C25 已记「## 覆盖」）。  
> 方法：复核 WP-021 T1–T9 对应当前树；`rg` 扫 `it.skip|describe.skip|it.todo|expect(true)`（0 命中）、basename 自指、className 实现细节、mock 被测邻接单元。  
> 建议口径：**删** = 去掉该用例/行；**留** = 保持；**放宽** = 改断言形态或合并实现，不整文件硬删。

### A. WP-021 T1–T9 复核（对应当前树）

| # | 来源 | 文件:行（当前） | 现状 | 建议 | 理由 |
|---|------|----------------|------|------|------|
| P1 | T1 | `r5-death-list.test.ts:44-46`；`r6:53-55`；`r9:70-72`；`session-seq-a:53-55`；`session-seq-d:42-44` — 共 5 条 `expect(basename(__filename)).toBe("…")` | **仍在**，未收敛 | **删**（5 条用例） | 同义反复假绿；不守任何产品不变量。替换方案见 WP-021 §2（non-vacuous：files.length 锚点、FORBIDDEN 非空、植入 fixture、扫到 .tsx）；替换与删除宜同批，否则门变秃 |
| P2 | T2 | `r5-death-list.test.ts:11` `partialText/partialReasoning` ⊂ `r9:13-18`；另与 `r4:11-18` 的 `tool_start/liveTools` 部分重叠 | **仍重叠** | **留**（合并不硬删） | 尊重 WP-021 desktop 交叉审：R5 是 ISSUE/回归编号锚。建议保留 R5 文件；在 R9 注释「含 R5 针」；或从 R9 去掉与 R5 完全重复的两针、保留 R5。**不**整删 R5 |
| P3 | T3 | 8 份 `walkTsFiles`：`r4:24`、`r5:13`、`r6:23`、`r9:26`、`seq-a:23`、`seq-d:12`、`serve-auth:22`、`model-selection:17` | **仍各自复制**；豁免口径仍漂移（r4/serve-auth 跳过 `*.test.*`；r5/r6/r9/model 用 `includes("death-list")`；seq-a/d 两者都跳） | **放宽**（抽共享 helper） | 非删测；抽 `web/src/test/deathListGate.ts`，豁免改精确正则 `/^[\w-]+-death-list\.test\.ts$/`，统一是否扫测试。可与 P1 同批 |
| P4 | T4 | `r9-death-list.test.ts:56-68`「accepts shared golden Item JSON fixtures」 | **仍弱**：只断言 `item.type` ∈ 枚举，未过 FE adapter | **放宽**（加强） | 倾向加强为 fixture→adapter→投影断言（跨语言锚值得留）；若不做加强再改 **删**。不建议无替代就删 |
| P5 | T5 | `model-selection-death-list.test.ts:54` `src.includes("modelId")` | **仍在**，近恒真 | **删**（仅此一行） | 几乎恒真；同用例 `:52-53` 负向针与 `:56` `Missing: ` 保留 |
| P6 | T6 | `serve-auth-death-list.test.ts:18-19`「Auth token (dev only)」「Save auth」 | **仍在**（负向针，禁旧 UI 文案回归） | **留** | 默认留；尊重交叉审反对删。换措辞可绕过，但去掉即失「旧 auth UI 原样复活」唯一文案门；成本极低。可选远期改成「组件/testid 不存在」仍属留门 |
| P7 | T7 | `PermissionModal.test.tsx:255-259` `flex-col` + `w-full` className | **仍在**；`justify-start` 已注释（ISSUE-008） | **放宽**（不硬删） | 尊重交叉审：与 `flex-col` 一并改为 role/布局语义（选项纵向、可点满宽容器），保留 ISSUE-008 回归意图；不直接删断言 |
| P8 | T8 | seq-a / seq-d 主扫描、r4、r6 主扫描 | **仍有价值** | **留** | 守 seq/持久化同构，Rust death_list_gate 有对应；仅随 P1 删其中 basename 子用例 |
| P9 | T9 | `debug-fold.test.tsx`；`PermissionModal.test.tsx:260-261` justify-start | **已解决**：debug-fold 文件不存在；justify-start 已改注释 | **留**（关闭待办） | 从裁剪待办移除，无需再动 |

### B. 额外扫描（T1–T9 之外）

| # | 分类 | 文件:行 | 现状 | 建议 | 理由 |
|---|------|---------|------|------|------|
| P10 | 实现细节 class | `TerminalPanel.test.tsx:88-92`、`:105-108`、`:136-139`、`:151-152`、`:164` 等大量 Tailwind/`--_dk-*` class 串 | 样式重构易假红；行为不变也红 | **放宽** | 改为 role/可见性/互斥态（active vs idle 文案或 aria）；保留 1–2 条关键交互断言即可 |
| P11 | 实现细节 class | `ComposerDock.test.tsx:64-68` `rounded-md` / `backdrop-blur-[12px]` / 禁 `amber`；`:113-127` flex 链 `min-h-0` | 玻璃主题契约有意锁定，但数值类名脆 | **放宽** | 负向针（禁 `fixed` 全屏、禁 amber 条）**留**；正向精确 blur/rounded **放宽**为 testid 或主题 token 存在性，避免像素级 class 锁死 |
| P12 | mock 邻接单元 | `AgentPanel.test.tsx:32-33` `vi.mock(PermissionModal)` → 桩卡片 | 测的是 fail-closed 身份壳，Ask 行为不经此文件 | **留** | 身份分类测本身有效；Ask 覆盖已由 PermissionModal + ComposerDock + turnStore 承担。勿在未补积分测前删 AgentPanel 套件。备注：不可拿本文件当 C20 证据 |
| P13 | 宽松豁免 | `r5:21`、`r6:30`、`r9:33`、`model-selection:24` 等 `name.includes("death-list")` | 产品文件名若含该子串会被静默豁免 | **放宽**（随 P3） | 并入共享 helper 的精确正则；非单独删测 |

### C. 扫描阴性结果（无需提案）

- `it.skip` / `describe.skip` / `it.todo` / `expect(true)` / `it.only`：`web/src` 测试下 **0 命中**。
- 空 `it(...){}`：未发现。

### D. 汇总（供裁决）

| 建议 | 条数 | 编号 |
|------|------|------|
| 删 | 2 | P1（5 条 basename 用例）、P5（1 行） |
| 留 | 5 | P2、P6、P8、P9、P12 |
| 放宽 | 6 | P3、P4、P7、P10、P11、P13 |

**T1–T9 结论摘要**：T1/T5 仍假绿/近恒真→删；T2/T6/T8 留（T2 合并不硬删，T6 负向文案门保留）；T3/T4/T7 放宽（helper / 加强 golden / class→语义）；T9 已关闭。额外 P10–P13 为 class 脆断言与 mock/豁免口径，均不在未裁决前改动。

**硬约束重申**：未过前端中层裁决前 **零删除**；不补 Step1 缺口新测；不碰 desktop/后端；不 commit；不 deliver（等中层收裁剪节）。

---

## 裁决与落地

> 前端中层裁决落地 · 2026-10-12 20:19 HKT · frontend（WP-045）  
> 只改 web 测试/辅助与本产物。未补 Step1 缺口新测。未碰 desktop/后端。未 commit。

### 逐条落地

| # | 裁决 | 做了什么 |
|---|------|----------|
| P1 | 删 | 删除 r5/r6/r9/seq-a/seq-d 共 5 条 `basename(__filename)` 自指用例及无用 `basename` import。**未**补 non-vacuous 替换（记后续） |
| P2 | 留 | 未动 R5；R9 文件头注释注明「含 R5 针」 |
| P3+P13 | 放宽 | 新增 `web/src/test/deathListGate.ts`（`collectTsFiles` / `scanNeedles` / `DEATH_LIST_GATE_RE`）；8 份 death-list 迁走本地 `walkTsFiles`；豁免改为精确正则 |
| P4 | 留不加强 | golden fixture 用例保留原断言；加强记后续 |
| P5 | 删一行 | `model-selection-death-list.test.ts` 去掉 `src.includes("modelId")`；`"unknown"` / `useConnectionStore` 负向与 `Missing: ` 保留 |
| P6 | 留 | serve-auth「Auth token (dev only)」「Save auth」负向针未动 |
| P7 | 放宽 | `PermissionModal.test.tsx`：`flex-col`/`w-full` → options list 含子 button、可点、**禁 `justify-start`（ISSUE-008）** |
| P8 | 留 | seq-a/d、r4、r6 主扫描保留（已迁 helper） |
| P9 | 留/关闭 | 无需再动 |
| P10 | 代表性放宽 | `TerminalPanel.test.tsx` active/idle：去掉 `--_dk-text-primary/secondary` 锁；保留「双行存在 + 非 opacity/surface-raised + hover-only bg」。其余脆 class（close 按钮 opacity、divider、resize hairline、justify-end）**列后续** |
| P11 | 放宽 | ComposerDock：正向 glass 改为断言含 `composerCardClass` 全 token + `permission-card` class；负向禁 `fixed` 全屏 / `border-l-2` / `amber` 保留 |
| P12 | 留 | AgentPanel PermissionModal mock 未动 |

### 改动文件

- 新增：`web/src/test/deathListGate.ts`
- 重写迁 helper：`web/src/api/{r4,r5,r6,r9,session-seq-a,session-seq-d,serve-auth,model-selection}-death-list.test.ts`
- 放宽：`web/src/components/PermissionModal.test.tsx`、`web/src/dockview/panels/ComposerDock.test.tsx`、`web/src/dockview/panels/TerminalPanel.test.tsx`
- 产物：本文件

### vitest（只读跑被改文件）

```
/exec-daemon/node node_modules/vitest/vitest.mjs run \
  src/api/*death-list*.test.ts \
  src/components/PermissionModal.test.tsx \
  src/dockview/panels/ComposerDock.test.tsx \
  src/dockview/panels/TerminalPanel.test.tsx
```

结果：**11 files / 45 tests passed**（Node 22.14.0）。

### 后续（本包不做）

1. **P1 non-vacuous**：files.length 锚点 / FORBIDDEN 非空 / 植入 fixture / 扫到 .tsx（见 WP-021 §2）
2. **P4**：golden fixture → FE adapter 投影断言
3. **P10 其余**：TerminalPanel close/divider/resize/justify-end 等 class 串
4. **Step1 缺口 C20–C25**：双队头 UI、无重放正面断言、snapshot 孤儿回归、turn start/finish 清队、多 session Ask 隔离
