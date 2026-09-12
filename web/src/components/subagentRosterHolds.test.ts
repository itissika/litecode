import { afterEach, describe, expect, it } from "vitest";

import {
  holdSubagentRoster,
  isSubagentRosterHeld,
  releaseSubagentRoster,
  resetSubagentRosterHolds,
} from "./subagentRosterHolds";

afterEach(() => {
  resetSubagentRosterHolds();
});

describe("subagentRosterHolds", () => {
  it("reports a child as held only between hold and release", () => {
    expect(isSubagentRosterHeld("child-a")).toBe(false);

    holdSubagentRoster("child-a");
    expect(isSubagentRosterHeld("child-a")).toBe(true);
    expect(isSubagentRosterHeld("child-b")).toBe(false);

    releaseSubagentRoster("child-a");
    expect(isSubagentRosterHeld("child-a")).toBe(false);
  });

  it("tolerates repeated hold / release (React remount cycles)", () => {
    holdSubagentRoster("child-a");
    holdSubagentRoster("child-a");
    releaseSubagentRoster("child-a");
    expect(isSubagentRosterHeld("child-a")).toBe(false);

    // A release without a hold is a no-op, not an error.
    releaseSubagentRoster("child-b");
    expect(isSubagentRosterHeld("child-b")).toBe(false);
  });
});
