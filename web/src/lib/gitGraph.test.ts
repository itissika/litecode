import { describe, expect, it } from "vitest";

import {
  clipLane,
  graphWidth,
  laneX,
  layoutGitGraph,
  maxLanesForWidth,
  GIT_GRAPH_LANE_WIDTH,
  GIT_GRAPH_PAD_X,
} from "./gitGraph";

describe("layoutGitGraph", () => {
  it("keeps a linear history on one lane", () => {
    const layout = layoutGitGraph([
      { sha: "a", parents: ["b"] },
      { sha: "b", parents: ["c"] },
      { sha: "c", parents: [] },
    ]);
    expect(layout.laneCount).toBe(1);
    expect(layout.rows.map((r) => r.lane)).toEqual([0, 0, 0]);
    expect(layout.rows[0]?.continueDown).toBe(true);
    expect(layout.rows[0]?.incoming).toBe(false);
    expect(layout.rows[1]?.incoming).toBe(true);
  });

  it("opens a second lane on merge and joins at the fork", () => {
    const layout = layoutGitGraph([
      { sha: "m", parents: ["c", "f"] },
      { sha: "f", parents: ["c"] },
      { sha: "c", parents: [] },
    ]);
    expect(layout.laneCount).toBe(2);
    expect(layout.rows[0]).toMatchObject({
      sha: "m",
      lane: 0,
      continueDown: true,
      merges: [{ from: 0, to: 1 }],
    });
    expect(layout.rows[1]).toMatchObject({
      sha: "f",
      lane: 1,
      incoming: true,
    });
    expect(layout.rows[1]?.merges).toEqual([{ from: 1, to: 0 }]);
    expect(layout.rows[2]).toMatchObject({ sha: "c", lane: 0 });
  });
});

describe("graph width clipping", () => {
  it("keeps at least one lane and clips overflow onto the last column", () => {
    expect(maxLanesForWidth(0)).toBe(1);
    expect(maxLanesForWidth(GIT_GRAPH_PAD_X + GIT_GRAPH_LANE_WIDTH + 200)).toBeGreaterThan(1);
    expect(clipLane(5, 3)).toBe(2);
    expect(clipLane(0, 1)).toBe(0);
    expect(graphWidth(8, 3)).toBe(GIT_GRAPH_PAD_X + 3 * GIT_GRAPH_LANE_WIDTH);
    expect(laneX(7, 3)).toBe(laneX(2, 3));
  });
});
