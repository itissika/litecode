import { describe, expect, it } from "vitest";

import { clipboardImageFiles, fittedSize, IMAGE_LONG_SIDE } from "./imageNormalize";

describe("fittedSize", () => {
  it("keeps an image that already fits", () => {
    expect(fittedSize(800, 600)).toEqual({ width: 800, height: 600 });
  });

  it("shrinks the long side to 1568 and keeps the ratio", () => {
    expect(fittedSize(3136, 1568)).toEqual({ width: IMAGE_LONG_SIDE, height: 784 });
    expect(fittedSize(1000, 4000)).toEqual({ width: 392, height: IMAGE_LONG_SIDE });
  });
});

describe("clipboardImageFiles", () => {
  it("reads image files and ignores text", () => {
    const image = new File([new Uint8Array([1])], "shot.png", {
      type: "image/png",
    });
    const files = clipboardImageFiles({
      items: [
        { kind: "string", type: "text/plain", getAsFile: () => null },
        { kind: "file", type: "image/png", getAsFile: () => image },
      ],
      files: [],
    } as unknown as DataTransfer);
    expect(files).toEqual([image]);
  });
});
