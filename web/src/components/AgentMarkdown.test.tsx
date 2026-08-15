import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { AgentMarkdown } from "./AgentMarkdown";

afterEach(cleanup);

describe("AgentMarkdown raw HTML boundary", () => {
  it("renders raw HTML as inert text instead of executable DOM", () => {
    const raw = [
      '<iframe srcdoc="<script>globalThis.pwned = true</script>"></iframe>',
      "<script>globalThis.pwned = true</script>",
      '<img src="x" onerror="globalThis.pwned = true">',
      '<div style="background:url(javascript:alert(1))">styled</div>',
      '<meta http-equiv="refresh" content="0;url=https://attacker.example">',
    ].join("\n\n");

    const { container } = render(<AgentMarkdown text={raw} />);

    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("[style]")).toBeNull();
    expect(container.querySelector("meta")).toBeNull();
    expect(container.textContent).toContain("<iframe");
    expect(container.textContent).toContain("<script>");
    expect(container.textContent).toContain("onerror=");
    expect(container.textContent).toContain("<meta");
  });

  it("keeps safe Markdown and GFM rendering", () => {
    const { container } = render(
      <AgentMarkdown
        text={[
          "## Safe heading",
          "",
          "**bold** and [docs](https://example.com)",
          "",
          "- [x] shipped",
          "",
          "| Name | Value |",
          "| --- | --- |",
          "| safe | yes |",
        ].join("\n")}
      />,
    );

    expect(screen.getByRole("heading", { name: "Safe heading" })).toBeTruthy();
    expect(screen.getByText("bold").tagName).toBe("STRONG");
    expect(screen.getByRole("link", { name: "docs" }).getAttribute("href")).toBe(
      "https://example.com",
    );
    expect(container.querySelector('input[type="checkbox"][checked]')).not.toBeNull();
    expect(screen.getByRole("table")).toBeTruthy();
  });
});
