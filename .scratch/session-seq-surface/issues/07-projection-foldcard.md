# 07: 转写投影与 FoldCard 只看该 seq

**What to build:** 人看到的列表是日志的投影：append-origin 按 seq，compact cut 是独立屏障，插在 replace `{start,end}` 之后。历史 process / FoldCard 会不会开，只看那些 seq 自己的 status，不看 session 级 `isRunning`。这是你最容易肉眼验收的一张：后续 turn 跑着时，旧折叠不得被顶开；cut 不得粘在上一轮气泡末尾。

**Blocked by:** 06: 前端唯一 Map&lt;seq, Event&gt;

**Status:** ready-for-agent

- [ ] `processGroupStreaming` / `isToolCallLive` 无 `turnActive`；输入是本组是否有 `in_progress` 的 seq（D1、D2）
- [ ] `ItemBubble` 不以 session `isRunning` 为 streaming 源（D3）；禁止 `isRunning && isLastBubble`
- [ ] cut 不 `push` 进上一 assistant 气泡（D5）；气泡 key = 组内 `min(seq)`，无 `assistant-after:user:`（D4）
- [ ] FoldCard `streaming` 不回落到「turn 还在跑」（D6）
- [ ] 探针：历史 completed process + `sessionRunning=true` → 推导 streaming false（G5）
- [ ] D1–D6 清零
