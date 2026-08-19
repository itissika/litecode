export const GIT_GRAPH_LANE_WIDTH = 12;
export const GIT_GRAPH_PAD_X = 8;
export const GIT_GRAPH_ROW_HEIGHT = 24;
/** Leave room for short SHA + a slice of the subject. */
export const GIT_GRAPH_RESERVED_TEXT = 88;

export type GraphCommit = {
  sha: string;
  parents: string[];
};

export type GraphMerge = {
  from: number;
  to: number;
};

export type GraphRow = {
  sha: string;
  lane: number;
  /** Lanes other than `lane` that carry a line through this row. */
  passing: number[];
  /** Incoming line from the previous row on this commit's lane. */
  incoming: boolean;
  /** First parent continues down the same lane. */
  continueDown: boolean;
  merges: GraphMerge[];
};

export type GitGraphLayout = {
  rows: GraphRow[];
  laneCount: number;
};

export function maxLanesForWidth(
  width: number,
  laneWidth = GIT_GRAPH_LANE_WIDTH,
  reservedText = GIT_GRAPH_RESERVED_TEXT,
  padX = GIT_GRAPH_PAD_X,
): number {
  const budget = Math.max(laneWidth, width - reservedText);
  return Math.max(1, Math.floor((budget - padX) / laneWidth));
}

export function clipLane(lane: number, maxLanes: number): number {
  const cap = Math.max(1, maxLanes);
  return Math.min(lane, cap - 1);
}

export function graphWidth(laneCount: number, maxLanes: number): number {
  const shown = Math.max(1, Math.min(laneCount, maxLanes));
  return GIT_GRAPH_PAD_X + shown * GIT_GRAPH_LANE_WIDTH;
}

export function laneX(lane: number, maxLanes: number): number {
  return GIT_GRAPH_PAD_X + clipLane(lane, maxLanes) * GIT_GRAPH_LANE_WIDTH + GIT_GRAPH_LANE_WIDTH / 2;
}

/** Newest-first topo log → column assignment (VS Code-style lanes). */
export function layoutGitGraph(commits: GraphCommit[]): GitGraphLayout {
  const rows: GraphRow[] = [];
  let active: (string | null)[] = [];
  let laneCount = 0;

  for (const commit of commits) {
    const incomingLanes = active.map((s) => s != null);
    let lane = active.findIndex((s) => s === commit.sha);
    if (lane === -1) {
      lane = active.findIndex((s) => s == null);
      if (lane === -1) {
        lane = active.length;
        active.push(commit.sha);
      } else {
        active[lane] = commit.sha;
      }
    }

    const passing: number[] = [];
    for (let i = 0; i < active.length; i++) {
      if (active[i] != null && i !== lane) passing.push(i);
    }

    const next = active.slice();
    for (let i = 0; i < next.length; i++) {
      if (next[i] === commit.sha) next[i] = null;
    }

    const merges: GraphMerge[] = [];
    let continueDown = false;
    const parents = commit.parents.filter(Boolean);

    parents.forEach((parent, pi) => {
      const occupied = next.findIndex((s) => s === parent);
      if (pi === 0) {
        if (occupied === -1) {
          next[lane] = parent;
          continueDown = true;
        } else if (occupied !== lane) {
          merges.push({ from: lane, to: occupied });
        }
        return;
      }
      if (occupied === -1) {
        let dest = next.findIndex((s) => s == null);
        if (dest === -1) {
          dest = next.length;
          next.push(parent);
        } else {
          next[dest] = parent;
        }
        merges.push({ from: lane, to: dest });
      } else {
        merges.push({ from: lane, to: occupied });
      }
    });

    rows.push({
      sha: commit.sha,
      lane,
      passing,
      incoming: incomingLanes[lane] === true,
      continueDown,
      merges,
    });

    active = next;
    while (active.length > 0 && active[active.length - 1] == null) active.pop();
    laneCount = Math.max(laneCount, active.length, lane + 1);
  }

  return { rows, laneCount: Math.max(laneCount, rows.length > 0 ? 1 : 0) };
}
