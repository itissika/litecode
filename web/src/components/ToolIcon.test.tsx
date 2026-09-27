import { cleanup, render } from "@testing-library/react";
import {
  GlobeIcon,
  MagnifyingGlassIcon,
  WrenchIcon,
} from "@phosphor-icons/react";
import { afterEach, describe, expect, it } from "vitest";

import { LitecodeMark } from "./LitecodeMark";
import { ToolIcon, glyphFor } from "./ToolIcon";

afterEach(() => {
  cleanup();
});

describe("tool glyph mapping", () => {
  it("uses the inline brand mark for the litecode workspace tool", () => {
    expect(glyphFor("litecode_workspace")).toBe(LitecodeMark);
    const { container } = render(
      <ToolIcon name="litecode_workspace" status="ok" />,
    );
    // Path data only — no webfont dependency in a 12px row.
    expect(
      container.querySelector("svg.tool-icon-glyph path")?.getAttribute("d"),
    ).toBeTruthy();
  });

  it("covers the search and network tools", () => {
    expect(glyphFor("grep")).toBe(MagnifyingGlassIcon);
    expect(glyphFor("session_search")).toBe(MagnifyingGlassIcon);
    expect(glyphFor("webfetch")).toBe(GlobeIcon);
    expect(glyphFor("websearch")).toBe(GlobeIcon);
  });

  it("reserves the generic wrench for unknown tools", () => {
    expect(glyphFor("mystery_tool")).toBe(WrenchIcon);
    expect(glyphFor("mcp_github")).not.toBe(WrenchIcon);
  });
});

describe("ToolIcon settle animation", () => {
  it("pops when streaming transitions true→false on ok", () => {
    const { container, rerender } = render(
      <ToolIcon name="write" status="ok" streaming />,
    );
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
    rerender(<ToolIcon name="write" status="ok" streaming={false} />);
    expect(container.querySelector(".tool-icon--pop")).toBeTruthy();
  });

  it("pops with the warn colour when streaming transitions true→false on warning", () => {
    const { container, rerender } = render(
      <ToolIcon name="write" status="warning" streaming />,
    );
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
    rerender(<ToolIcon name="write" status="warning" streaming={false} />);
    expect(container.querySelector(".tool-icon--pop")).toBeTruthy();
    expect(container.querySelector(".tool-icon--warn")).toBeTruthy();
  });

  it("stays static when mounted with streaming=false (no transition)", () => {
    const { container } = render(
      <ToolIcon name="write" status="ok" streaming={false} />,
    );
    expect(container.querySelector(".tool-icon--pop")).toBeNull();
  });

  it("plays fail animation when streaming transitions true→false on failed", () => {
    const { container, rerender } = render(
      <ToolIcon name="write" status="failed" streaming />,
    );
    expect(container.querySelector(".tool-icon--fail-anim")).toBeNull();
    rerender(<ToolIcon name="write" status="failed" streaming={false} />);
    expect(container.querySelector(".tool-icon--fail-anim")).toBeTruthy();
  });
});
