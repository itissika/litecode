import type { CatalogProviderDto } from "../../../api/settings";

/**
 * WP-114: single billing partition for Settings → Providers.
 * Only this table — no dual-format / billing_kind fallback.
 *
 * WP-113 mid final (2026-10-13): replaced moonshot-plan/payg & minimax-plan/payg
 * placeholders with moonshot-coding / moonshot / minimax-token / minimax;
 * opencode-go → Plan; opencode + commandcode → API.
 */
const PLAN_PROVIDER_IDS: ReadonlySet<string> = new Set([
  "ark-coding",
  "ark-agent",
  "tencent-token",
  "aliyun-token",
  "moonshot-coding", // Kimi Code
  "minimax-token", // M Plan
  "opencode-go", // monthly subscription
]);

// Everything not in PLAN_PROVIDER_IDS is API (openai, deepseek, mimo, zhipu,
// moonshot, minimax, opencode, commandcode, and any unknown id).
type ProviderBillingGroup = "plan" | "api";

export function providerBillingGroup(id: string): ProviderBillingGroup {
  return PLAN_PROVIDER_IDS.has(id) ? "plan" : "api";
}

export function partitionProviders(providers: CatalogProviderDto[]): {
  plan: CatalogProviderDto[];
  api: CatalogProviderDto[];
} {
  const plan: CatalogProviderDto[] = [];
  const api: CatalogProviderDto[] = [];
  for (const p of providers) {
    if (providerBillingGroup(p.id) === "plan") plan.push(p);
    else api.push(p);
  }
  return { plan, api };
}
