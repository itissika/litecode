# WP-114 方案（前端-web；只改 web/，不 commit，未批不 start）

口径：WP-110-frontend.md。Settings → Providers 列表按计费形态拆成 **Plan** / **API 按量** 两组。卡片形态、密钥保存、其它 Settings / 选模 UI 不变。只维护一套分区规则，不做新旧双格式兼容。

## Goal
visible providers 分成两组渲染；各组保留现有两列网格 + `ProviderCard`。

## 改动点（批后实施）
1. **`web/src/dockview/panels/settings/ProvidersSection.tsx`**（约 :81–141）
   - `providers = filter(visible)` 后，调用纯函数 `partitionProviders(providers)` → `{ plan, api }`。
   - 渲染两段：各一段组标题（建议文案 `Plan` / `API`，或短中文「套餐 Plan」/「按量 API」— 实施时定一种）+ 现有 `grid grid-cols-2` + `ProviderCard`。
   - 空组：该组不渲染标题与网格（或渲染标题 + 空占位；实施时选「有卡才出标题」以免空标题）。
   - 密钥草稿 / autosave / configured 灰态 / `key={provider.id}` **逻辑不动**，只改列表布局。

2. **抽出纯函数 + 规则表**（同文件顶部或邻接 `providerBilling.ts`）
   - 常量 `PLAN_PROVIDER_IDS: ReadonlySet<string>`：
     - 现网：`ark-coding`、`ark-agent`、`tencent-token`、`aliyun-token`
     - 占位（WP-113 最终 id 以中层转发 / `WP-110-provider.md` 为准；方案先用）：`moonshot-plan`、`minimax-plan`（若最终 id 不同，规则表改一次）
   - 常量 `API_PROVIDER_IDS`（显式列出便于测）：`openai`、`deepseek`、`mimo`、`zhipu`；以及占位 `moonshot-payg`、`minimax-payg`
   - **`opencode` / `opencode-go` / `commandcode`：桶待 backend_mid 确认** — 方案占位，**不自创第二套格式**；确认前不进 PLAN 表（若出现在 catalog，走「未知 → API」默认，或确认后只改规则表一次）
   - `partitionProviders(list)`：`id ∈ PLAN_PROVIDER_IDS` → plan；其余（含未知与确认前的网关 id）→ api。**单一路径，不读后端 billing_kind 字段**（本波 catalog 无该字段）。

3. **`ProviderLogos.tsx`**
   - 仅当新 id 缺 logo 时补映射或走现有 fallback；**不挡分组**。无新 id 时可不动。

4. **不改**：`ModelsSection` / `ModelSwitcher` / Agents / 密钥 PUT；后端 catalog/config；desktop 代码。

## 测试（`ProvidersSection.test.tsx`，vitest `/exec-daemon/node` v22）
1. plan 四家（ark-coding / ark-agent / tencent-token / aliyun-token）在 Plan 组。
2. openai / deepseek / mimo / zhipu 在 API 组。
3. 两组各有可见 section 标题（或等价 a11y：`role="region"` / `aria-labelledby` / heading）。
4. 模拟 `moonshot-plan` + `moonshot-payg`（或 WP-113 最终 id）分别落入 Plan / API。
5. 未知 id → API 组。
6. 回归：key 草稿/autosave、configured 灰态、`visible:false` 不展示。
7. `tsc -b` 0；lint N/A。

另：`partitionProviders` 可单测规则表（含未知默认、占位 moonshot 对）。

## 待中层 / backend 确认（不阻塞本方案提交）
- `opencode` / `opencode-go` / `commandcode` 归 Plan 还是 API — 确认后只改规则表。
- Moonshot / MiniMax 最终 provider id — 对齐 `WP-110-provider.md` 后改规则表 + 测 4。

## desktop
内嵌 web，不改；静态核对 `git status desktop`。

## 验收
测 1–7 绿；diff 仅 web/（+ ops 产物）；未 commit。

## 不在范围
Models/ModelSwitcher 再分组；后端；双格式兼容；commit/push。

## 中层转发 WP-113 定稿（2026-10-13 HKT · frontend_mid）

backend_mid 已锁定最终 provider id，**请改规则表一次**（与 `WP-110-frontend.md` 同步；勿继续用占位名）：

- **Plan**：`ark-coding`、`ark-agent`、`tencent-token`、`aliyun-token`、`moonshot-coding`、`minimax-token`、`opencode-go`
- **API**：`openai`、`deepseek`、`mimo`、`zhipu`、`moonshot`、`minimax`、`opencode`、`commandcode`；**未知 → API**
- **作废占位**：`moonshot-plan` / `moonshot-payg` / `minimax-plan` / `minimax-payg` 勿再用
- OpenCode 桶已定（`opencode-go`→Plan；`opencode`/`commandcode`→API），不再「待 backend 确认」
- 测 4 请改用最终 id 对：`moonshot-coding`↔`moonshot`、`minimax-token`↔`minimax`；并覆盖 OpenCode 三家落点

（本段为中层追加说明；上方原方案正文若仍写占位/待确认，以本段为准。）

---
## 实施记录（WP-114；2026-10-13 HKT）
### WP-113 中层定稿替换（一次改规则表，无双格式）
- **Plan：** `ark-coding`、`ark-agent`、`tencent-token`、`aliyun-token`、`moonshot-coding`（Kimi Code）、`minimax-token`（M Plan）、`opencode-go`（月订阅）
- **API 显式：** `openai`、`deepseek`、`mimo`、`zhipu`、`moonshot`、`minimax`、`opencode`、`commandcode`；未知 → API
- 已删除占位：`moonshot-plan` / `moonshot-payg` / `minimax-plan` / `minimax-payg`
- 测 4 改为 `moonshot-coding` + `moonshot`

### 改动文件
- `web/src/dockview/panels/settings/providerBilling.ts`（新）：`PLAN_PROVIDER_IDS` / `API_PROVIDER_IDS` / `partitionProviders` / `providerBillingGroup` — 单一规则表
- `web/src/dockview/panels/settings/ProvidersSection.tsx`：visible 列表拆 Plan / API 两组（有卡才出标题）；各组两列网格 + ProviderCard；密钥草稿/autosave/configured 灰态不变
- `web/src/dockview/panels/settings/ProvidersSection.test.tsx`：测 1–6 + partitionProviders 单测；`ProviderLogos` 未改（无新 logo 需求）

### 证据（/exec-daemon/node v22.14.0）
- 相关：ProvidersSection.test.tsx → 14 passed
- 全量：Test Files 174 passed，Tests 1295 passed
- `tsc -b` exit 0；lint N/A
- 未改 Models / 密钥 PUT / 后端 / desktop；未 commit
