/**
 * Child sessions an expanded dock "Workers" roster card is currently rendering.
 *
 * `ensureSubscribe` / `unsubscribeSession` are not refcounted, yet the roster
 * card and the child's own `agent-<childId>` dock tab routinely hold the SAME
 * subscription (expand a card, then open the child as a tab). Closing one side
 * must therefore not tear down what the other still renders.
 *
 * The child→tab direction is guarded in `SubagentCardBody`; this registry
 * provides the tab→child direction: the card registers its child while mounted,
 * `AgentPanel`'s unmount checks the registry before releasing.
 */
const held = new Set<string>();

export function holdSubagentRoster(childSessionId: string): void {
  held.add(childSessionId);
}

export function releaseSubagentRoster(childSessionId: string): void {
  held.delete(childSessionId);
}

export function isSubagentRosterHeld(childSessionId: string): boolean {
  return held.has(childSessionId);
}

/** Test seam: clear every hold. */
export function resetSubagentRosterHolds(): void {
  held.clear();
}
