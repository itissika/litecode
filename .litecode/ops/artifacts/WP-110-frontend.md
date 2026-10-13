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
- **目录现状**（`src/provider_catalog/default_catalog.toml`）：尚无 Moonshot/MiniMax；现有 plan 名产品为 `ark-coding` / `ark-agent` / `tencent-token` / `aliyun-token`；按量/直连为 `openai` / `deepseek` / `mimo` / `zhipu`；网关 `opencode` / `opencode-go` / `commandcode` 桶待与 backend_mid 对齐。
- **不改**：`ModelsSection.tsx`（按已配置 provider 折卡）、`ModelSwitcher.tsx`（按 `provider_id` 聚模型）— 属「其它 UI 不变」。

## 分组规则（与 WP-111/113 对齐）
1. **本波默认**：前端用**稳定规则**分区（实现自定：allowlist 和/或 id/name/endpoint 启发式），与后端「一家 plan、一家 payg」双 provider 命名对齐（WP-110-backend：命名类比 ark-coding / ark-agent）。
2. **临时归桶（现 catalog）**
   - **Plan**：`ark-coding`、`ark-agent`、`tencent-token`、`aliyun-token`
   - **API 按量**：`openai`、`deepseek`、`mimo`、`zhipu`
   - **待 backend_mid 确认**：`opencode`、`opencode-go`、`commandcode`（网关/订阅，先占位；定稿后只改规则表，不引入第二套格式）
3. **Moonshot / MiniMax**：WP-113 落地后若有 plan+按量 → **两个 provider id**；分别落入 Plan / API 组。最终 id 以 backend_mid 私信 / `WP-110-provider.md` 为准；前端规则表跟上一次。
4. **可选后续**：若后端日后在 catalog / `LlmProviderView` 增加 `billing_kind: "plan"|"api"`，前端可改读该字段；**本包不阻塞**，可先靠前端规则上线。
5. **单一格式**：只维护一套分区表/规则；不做「有字段用字段、无字段用启发式」双路径长期并存。

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
4. 模拟 moonshot-plan + moonshot-payg（或 WP-113 最终 id）分别落入正确组。
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
- WP-113 双端点 id 出现后规则表已纳入且测覆盖
- 与 backend_mid 对过 OpenCode/commandcode 桶与最终 Moonshot/MiniMax id（可在联调笔记写明）
- 未 commit

## 待确认
1. **backend_mid**：`opencode` / `opencode-go` / `commandcode` 归 Plan 还是 API 按量？
2. **backend_mid / WP-113**：Moonshot、MiniMax 最终 provider id（有 plan 则两套）— 对齐后改规则表一次。
3. **GM**（可选）：组标题中英偏好；未知 id 默认进 API 是否认可。
