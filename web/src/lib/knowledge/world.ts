/** Canvas position. Stored on nodes and treated as the authority. */
export interface WorldPoint {
  x: number;
  y: number;
}

export interface FlowPointNode {
  id: string;
  type?: string;
  parentId?: string;
  position: WorldPoint;
}

export interface PositionWrite {
  id: string;
  x: number;
  y: number;
}

/** Parent-relative flow position from a world position and the parent frame. */
export function worldToFlow(
  world: WorldPoint,
  parentWorld: WorldPoint | null,
): WorldPoint {
  if (!parentWorld) return { x: world.x, y: world.y };
  return { x: world.x - parentWorld.x, y: world.y - parentWorld.y };
}

/** World position by adding each ancestor's flow position, nearest parent first. */
export function flowToWorld(
  position: WorldPoint,
  ancestors: readonly WorldPoint[],
): WorldPoint {
  let x = position.x;
  let y = position.y;
  for (const ancestor of ancestors) {
    x += ancestor.x;
    y += ancestor.y;
  }
  return { x, y };
}

function ancestorsOf(
  node: FlowPointNode,
  byId: ReadonlyMap<string, FlowPointNode>,
): WorldPoint[] {
  const ancestors: WorldPoint[] = [];
  let parentId = node.parentId;
  const seen = new Set<string>();
  while (parentId && !seen.has(parentId)) {
    seen.add(parentId);
    const parent = byId.get(parentId);
    if (!parent) break;
    ancestors.push(parent.position);
    parentId = parent.parentId;
  }
  return ancestors;
}

/** World position of one flow node, walking parent frames. */
export function worldFromFlowNode(
  node: FlowPointNode,
  byId: ReadonlyMap<string, FlowPointNode>,
): WorldPoint {
  return flowToWorld(node.position, ancestorsOf(node, byId));
}

function descendantCards(
  rootId: string,
  nodes: readonly FlowPointNode[],
): FlowPointNode[] {
  const children = new Map<string, FlowPointNode[]>();
  for (const node of nodes) {
    if (!node.parentId) continue;
    const list = children.get(node.parentId);
    if (list) list.push(node);
    else children.set(node.parentId, [node]);
  }
  const cards: FlowPointNode[] = [];
  const stack = [...(children.get(rootId) ?? [])];
  const seen = new Set<string>();
  while (stack.length > 0) {
    const current = stack.pop();
    if (!current || seen.has(current.id)) continue;
    seen.add(current.id);
    if (current.type === "knowledge") cards.push(current);
    else stack.push(...(children.get(current.id) ?? []));
  }
  return cards;
}

/**
 * World coordinates to write when a drag ends.
 * A card writes its own world position. A folder writes every card inside it,
 * so the derived frame stays where the folder was dropped.
 */
export function worldWritesForPositionChanges(
  changes: readonly {
    type: string;
    id?: string;
    dragging?: boolean;
    position?: WorldPoint;
  }[],
  fitted: readonly FlowPointNode[],
): PositionWrite[] {
  const byId = new Map(fitted.map((node) => [node.id, node]));
  const writes = new Map<string, PositionWrite>();
  for (const change of changes) {
    if (
      change.type !== "position" ||
      change.dragging !== false ||
      !change.position ||
      change.id == null
    ) {
      continue;
    }
    const moved = byId.get(change.id);
    if (!moved) continue;
    const targets =
      moved.type === "knowledgeFolder"
        ? descendantCards(moved.id, fitted)
        : moved.type === "knowledge"
          ? [moved]
          : [];
    for (const target of targets) {
      const world = worldFromFlowNode(target, byId);
      writes.set(target.id, {
        id: target.id,
        x: Math.round(world.x),
        y: Math.round(world.y),
      });
    }
  }
  return [...writes.values()];
}
