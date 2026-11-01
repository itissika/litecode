import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { ListRow } from "./ListRow";

afterEach(cleanup);

describe("ListRow", () => {
  it("computes tree padding from depth and draws guides", () => {
    const { container } = render(
      <ListRow depth={2} guides>
        row
      </ListRow>,
    );
    const row = container.firstElementChild as HTMLElement;
    expect(row.style.paddingLeft).toBe("32px");
    expect(container.querySelectorAll("[aria-hidden]")).toHaveLength(2);
  });

  it("supports the dense source-control rows", () => {
    const { container } = render(
      <ListRow size="tree-compact" inset={20} transition={false}>
        row
      </ListRow>,
    );
    const row = container.firstElementChild as HTMLElement;
    expect(row.className).toContain("text-dk-xs");
    expect(row.style.paddingLeft).toBe("20px");
    expect(row.style.transitionProperty).toBe("none");
  });

  it("draws the inside drop outline", () => {
    const { container } = render(
      <ListRow drop="inside" dropTone="outline">
        row
      </ListRow>,
    );
    const row = container.firstElementChild as HTMLElement;
    expect(row.className).toContain("outline-(--_dk-accent-hover)");
  });

  it("draws before/after insertion lines", () => {
    const { container } = render(<ListRow drop="before">row</ListRow>);
    const row = container.firstElementChild as HTMLElement;
    expect(row.querySelector("[aria-hidden]")).toBeTruthy();
    expect(row.querySelector(".top-0")).toBeTruthy();
  });
});
