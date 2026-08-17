import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { VersionTags } from "./VersionTags";

describe("VersionTags", () => {
  afterEach(() => {
    cleanup();
  });

  it("shows dev channel tag and version", () => {
    render(<VersionTags channelRaw="dev" version="0.1.4" />);
    screen.getByText("Dev");
    screen.getByText("v0.1.4");
  });

  it("shows nightly channel tag and version", () => {
    render(<VersionTags channelRaw="nightly" version="0.1.4" />);
    screen.getByText("Nightly");
    screen.getByText("v0.1.4");
  });

  it("hides channel tag for official builds but keeps version", () => {
    render(<VersionTags channelRaw="official" version="0.1.4" />);
    expect(screen.queryByText("Official")).toBeNull();
    screen.getByText("v0.1.4");
  });
});
