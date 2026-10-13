# WP-112 方案（frontend_mid · split）

- 父包：WP-110（GM，Andy 已批）。并行包：WP-111（backend_mid → WP-113 provider）。
- 审批：gm。产物：本文件 + 子包产物 `WP-110-web.md`。
- **GM 未批 plan 不 start；不 commit。只支持一种分组规则，不做新旧双格式兼容。**

## Goal（只做一项）
Settings → **Providers** 列表按计费形态分成两组：**Plan** 一组、**API 按量** 一组。卡片形态、密钥保存、其它 Settings / 选模 UI **不变**。

## 现状落点（只读核实，2026-10-13 HKT）
- **Providers 页**：`web/src/dockview/panels/settings/ProvidersSection.tsx`
  - `providers = (llm?.providers ?? []).filter(p => p.visible)`（约 :81-84）
  - 渲染为 **扁平** `grid grid-cols-2`（约 :129-141），无分组标题。
  - 单卡：`ProviderCard` — logo + `name` + API key 字段；`key={provider.id}`。
- **DTO**：`web/src/api/settings.ts` `CatalogProviderDto`（约 :58-69）— `id/name/visible/configured/masked_api_key/endpoint/endpoint_type/models`；**无** `billing_kind` / group 字段。
- **后端投影**：`src/config/settings_writer.rs` `LlmProviderView` 同上，无分组字段。
- **目录现状**（`src/provider_catalog/default_catalog.toml` + WP-113）：plan 名产品含 `ark-coding` / `ark-agent` / `tencent-token` / `aliyun-token` / `moonshot-coding` / `minimax-token` / `opencode-go`；按量/直连/聚合含 `openai` / `deepseek` / `mimo` / `zhipu` / `moonshot` / `minimax` / `opencode` / `commandcode`。分组规则已由 backend_mid **2026-10-13 HKT 锁定**（见下节）。
- **不改**：`ModelsSection.tsx`（按已配置 provider 折卡）、`ModelSwitcher.tsx`（按 `provider_id` 聚模型）— 属「其它 UI 不变」。

## 分组规则（与 WP-111/113 对齐 · backend_mid 2026-10-13 定稿）
1. **本波默认**：前端用**稳定规则**分区（实现自定：allowlist），与后端「一家 plan、一家 payg」双 provider 命名对齐（WP-110-backend：命名类比 ark-coding / ark-agent）。
2. **定稿归桶**（backend_mid 私信 + WP-113，**2026-10-13 HKT 锁定**）
   - **Plan**：`ark-coding`、`ark-agent`、`tencent-token`、`aliyun-token`、`moonshot-coding`、`minimax-token`、`opencode-go`
   - **API 按量**：`openai`、`deepseek`、`mimo`、`zhipu`、`moonshot`、`minimax`、`opencode`、`commandcode`
   - **未知 id → API 按量**（默认，避免 Plan 误收）
3. **Moonshot / MiniMax**：`moonshot-coding`（Kimi Code）/ `minimax-token`（M Plan）→ Plan；`moonshot` / `minimax`（按量）→ API。占位 id `moonshot-plan` / `moonshot-payg` / `minimax-plan` / `minimax-payg` **作废，勿再用**。
4. **OpenCode 系**：`opencode-go` → Plan（月订阅套餐）；`opencode`、`commandcode` → API（按量/聚合 PAYG）。已不再「待 backend 确认」。
5. **可选后续**：若后端日后在 catalog / `LlmProviderView` 增加 `billing_kind: "plan"|"api"`，前端可改读该字段；**本包不阻塞**，可先靠前端规则上线。
6. **单一格式**：只维护一套分区表/规则；不做「有字段用字段、无字段用启发式」双路径长期并存。

## 子包
| 子包 | owner | 域 | 产物 |
|------|-------|----|------|
| **WP-114** | `frontend`（前端-web） | client-ux | `WP-110-web.md` |

## web 落点（批后由子包实施）
- `ProvidersSection.tsx`：visible providers 分成 Plan / API 两组，各组保留现有两列网格与 `ProviderCard`；组标题文案建议 `Plan` / `API`（或中文「套餐 Plan」/「按量 API」— 实现自定，保持简短）。
- 抽出纯函数（如 `partitionProviders`）便于单测；规则表与 WP-113 新 id 同文件或邻接常量。
- `ProvidersSection.test.tsx`：覆盖两组标题出现、已知 plan/api id 落点、未知 id 默认桶（需在方案/测试中写明默认：建议落入 API 或单独「其它」— **本包默认未知进 API 按量**，避免 Plan 误收）。
- `ProviderLogos.tsx`：若新 id 无 logo，可补或走 fallback（不挡分组）。
- **不改** Models / ModelSwitcher / Agents / 密钥 PUT 语义。

## 测试清单（vitest）
1. 现有 plan 四家出现在 Plan 组。
2. openai/deepseek/mimo/zhipu 出现在 API 组。
3. 两组各有可见 section 标题（或等价 a11y 标记）。
4. 模拟 `moonshot-coding` + `moonshot`（及 `minimax-token` + `minimax`）分别落入 Plan / API；`opencode-go`→Plan，`opencode`/`commandcode`→API。
5. 未知 id → API 组（默认）。
6. 回归：key 草稿/autosave、configured 灰态、`visible:false` 仍不展示。
7. `tsc -b` 0；lint N/A（web 无 lint 工具）。

## desktop
Settings 为 web 内嵌；预计 **不改 desktop**。静态核对即可（`git status desktop` 应干净）；真机 Andy 验。

## 不在范围
- ModelsSection / ModelSwitcher 再分组
- 后端 catalog / config 编辑（WP-111/113）
- 双格式兼容、commit / push

## 验收标准
- 子包测 1–7 绿；中层交叉审 diff 仅 web/
- Providers 页可见 **Plan** 与 **API 按量** 两组；现网 plan/api 归桶正确
- WP-113 定稿 id 已纳入规则表且测覆盖（Plan 含 moonshot-coding/minimax-token/opencode-go；API 含 moonshot/minimax/opencode/commandcode）
- OpenCode / Moonshot / MiniMax 桶已与 backend_mid 锁定（2026-10-13），无需再占位
- 未 commit

## 已锁定（原「待确认」关闭）
1. ~~OpenCode 桶~~ → `opencode-go` Plan；`opencode` / `commandcode` API（backend_mid 2026-10-13）。
2. ~~Moonshot / MiniMax id~~ → Plan: `moonshot-coding` / `minimax-token`；API: `moonshot` / `minimax`。
3. **GM**（可选，不阻塞）：组标题中英偏好；未知 id 默认进 API 已按合同执行。

---
## 中层交叉审（WP-114；2026-10-13 HKT · frontend_mid，仓库恢复至 e408fe4 后重验）
- 规则表单一（providerBilling.ts）：Plan = ark-coding/ark-agent/tencent-token/aliyun-token/moonshot-coding/minimax-token/opencode-go；API = openai/deepseek/mimo/zhipu/moonshot/minimax/opencode/commandcode；未知→API。与 default_catalog.toml id 一致；占位 id 无残留（仅注释提及）。
- ProvidersSection：Plan / API 两组 + 两列网格 + ProviderCard；Models/ModelSwitcher/密钥 PUT/后端/desktop 未改。
- 证据（node v22.14.0；恢复后 npm ci 重装 node_modules）：ProvidersSection.test.tsx 14/14；web 全量 174 文件 / 1295/1295；tsc -b 0；lint N/A。
- desktop：Settings 为内嵌 web，无 desktop 代码改动，仅静态；Andy 真机验。未 commit。
- 结论：通过，WP-114 封 done。
