import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { FoldCard } from "./FoldCard";
import { clearFoldCardOpen } from "./foldCardState";

it("debug child dispatch", () => {
  clearFoldCardOpen("dbg");
  render(<FoldCard id="dbg:1" label="bash">body</FoldCard>);
  const h = screen.getByRole("button", { name: "bash" });
  const path = h.querySelector("path")!;
  path.dispatchEvent(new Event("webkitAnimationEnd", { bubbles: true }));
  console.log("children mounted after child dispatch:", screen.queryByText("body") !== null);
  console.log("aria:", h.getAttribute("aria-expanded"));
  expect(true).toBe(true);
});
