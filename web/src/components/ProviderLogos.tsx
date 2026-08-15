import type { JSX } from "react";

import { DeepSeekLogo } from "./providerLogos/DeepSeekLogo";
import { OpenAILogo } from "./providerLogos/OpenAILogo";
import { XiaomiLogo } from "./providerLogos/XiaomiLogo";

const LOGOS: Record<string, () => JSX.Element> = {
  deepseek_responses: DeepSeekLogo,
  openai_responses: OpenAILogo,
  mimo_responses: XiaomiLogo,
};

/**
 * 14px provider logo. Inline SVG with fill="currentColor", so the
 * mark inherits the surrounding text color (theme tokens) — no mask
 * tint needed, follows dark/light theme and hover states automatically.
 */
export function ProviderLogo({ adapterId }: { adapterId?: string }) {
  const Logo = adapterId ? LOGOS[adapterId] : undefined;
  if (!Logo) return null;
  return (
    <span className="inline-block h-3.5 w-3.5 shrink-0">
      <Logo />
    </span>
  );
}
