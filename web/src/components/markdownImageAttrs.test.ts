import { describe, expect, it } from "vitest";

import { fillMarkdownImageAttrs } from "./markdownImageAttrs";

describe("fillMarkdownImageAttrs", () => {
  it("turns a missing image title into an empty caption string", () => {
    const tree = {
      type: "root",
      children: [
        {
          type: "image-block",
          url: "a.png",
          alt: "shot",
          title: null,
        },
        {
          type: "paragraph",
          children: [
            {
              type: "image",
              url: "b.png",
              alt: null,
              title: null,
            },
          ],
        },
      ],
    };

    fillMarkdownImageAttrs(tree);

    expect(tree.children[0]?.title).toBe("");
    expect(tree.children[0]?.alt).toBe("shot");
    expect(tree.children[1]?.children?.[0]?.title).toBe("");
    expect(tree.children[1]?.children?.[0]?.alt).toBe("");
  });
});
