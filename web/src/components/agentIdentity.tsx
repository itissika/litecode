import { Robot, UsersThree } from "@phosphor-icons/react";

import type { AgentRole } from "../api/settings";

// Deterministic, stable color per agent id. Shared across every consumer
// (chatinput picker, settings agent dropdown, type switch) so the same agent
// always renders the same hue regardless of list order. Replaces the old
// index-based coloring that lived only inside AgentChatInput.
const AGENT_HUES = [
  "hsl(210 80% 55%)",
  "hsl(160 70% 50%)",
  "hsl(25 85% 55%)",
  "hsl(290 60% 58%)",
  "hsl(45 80% 50%)",
  "hsl(340 70% 55%)",
  "hsl(180 65% 48%)",
  "hsl(5 75% 55%)",
];

export function agentColor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h * 31 + id.charCodeAt(i)) >>> 0;
  }
  return AGENT_HUES[h % AGENT_HUES.length];
}

// Single source for the agent-type glyph. primary → Robot, subagent →
// UsersThree. The icon is optionally tinted with the agent's color so one
// glyph carries both the type (shape) and the auto-assigned color (tint),
// replacing the previous colored dot + text-suffix approaches.
export function AgentTypeIcon({
  role,
  color,
  size = 12,
}: {
  role: AgentRole;
  color?: string;
  size?: number;
}) {
  const Glyph = role === "subagent" ? UsersThree : Robot;
  return <Glyph size={size} aria-hidden style={color ? { color } : undefined} />;
}
