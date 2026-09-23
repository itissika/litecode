import type { JSX } from "react";

import { DeepSeekLogo } from "./providerLogos/DeepSeekLogo";
import { OpenAILogo } from "./providerLogos/OpenAILogo";
import { XiaomiLogo } from "./providerLogos/XiaomiLogo";

/** Catalog provider id → brand mark. */
export const PROVIDER_LOGOS: Record<string, () => JSX.Element> = {
  openai: OpenAILogo,
  deepseek: DeepSeekLogo,
  mimo: XiaomiLogo,
};

/** Neutral mark for providers without a brand asset (ark-coding, opencode, …). */
function NeutralLogo(): JSX.Element {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      aria-hidden="true"
    >
      <circle cx="7" cy="7" r="5.25" strokeWidth="1.25" />
      <circle cx="7" cy="7" r="1.75" fill="currentColor" stroke="none" />
    </svg>
  );
}

/**
 * 14px provider logo keyed by **catalog provider id** (`openai`, `deepseek`,
 * `mimo`, …). Inline SVG with fill="currentColor", so the mark inherits the
 * surrounding text color (theme tokens) — follows dark/light and hover states.
 * Unknown ids fall back to a neutral mark rather than rendering nothing.
 */
export function ProviderLogo({ providerId }: { providerId?: string }) {
  const Logo = providerId ? PROVIDER_LOGOS[providerId] : undefined;
  return (
    <span
      className="inline-block h-3.5 w-3.5 shrink-0"
      title={providerId || undefined}
      data-provider-logo={Logo ? "brand" : "neutral"}
    >
      {Logo ? <Logo /> : <NeutralLogo />}
    </span>
  );
}
