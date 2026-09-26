import { describe, expect, it } from "vitest";

import { fileKindFromPath } from "./fileKind";

describe("fileKindFromPath", () => {
  it("keeps source and unknown extensions as text", () => {
    expect(fileKindFromPath("src/main.rs")).toBe("text");
    expect(fileKindFromPath("notes")).toBe("text");
    expect(fileKindFromPath(".gitignore")).toBe("text");
    expect(fileKindFromPath("data.csv")).toBe("text");
  });

  it("classifies previews and known binaries without a text read", () => {
    expect(fileKindFromPath("shot.PNG")).toBe("image");
    expect(fileKindFromPath("doc.pdf")).toBe("pdf");
    expect(fileKindFromPath("a/theme.mp3")).toBe("audio");
    expect(fileKindFromPath("clip.mp4")).toBe("video");
    expect(fileKindFromPath("app.sqlite")).toBe("sqlite");
    expect(fileKindFromPath("pack.zip")).toBe("binary");
    expect(fileKindFromPath("mod.wasm")).toBe("binary");
  });
});
