import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SearchResultGroupCard, type SearchResultGroup } from "./SearchResults";

describe("SearchResultGroupCard", () => {
  it("shows server match_count and opens the title without toggling fold", () => {
    const onOpenTitle = vi.fn();
    const onOpenHit = vi.fn();
    const group: SearchResultGroup = {
      key: "sid",
      title: "01ABCDEF",
      subtitle: "path.md",
      matchCount: 12,
      onOpenTitle,
      lines: [
        { id: "h1", lineLabel: "3", text: "needle here", onOpen: onOpenHit },
      ],
    };
    render(<SearchResultGroupCard group={group} />);
    expect(screen.getByText("12")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "01ABCDEF" }));
    expect(onOpenTitle).toHaveBeenCalledTimes(1);
    expect(onOpenHit).not.toHaveBeenCalled();
  });
});
