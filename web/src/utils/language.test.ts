import { describe, expect, it } from "vitest";

import { languageFromPath } from "./language";

describe("languageFromPath", () => {
  it("keeps existing mappings", () => {
    expect(languageFromPath("src/main.rs")).toBe("rust");
    expect(languageFromPath("App.tsx")).toBe("typescript");
  });

  it("maps shader family extensions", () => {
    expect(languageFromPath("Foo.shader")).toBe("shaderlab");
    expect(languageFromPath("Lighting.hlsl")).toBe("hlsl");
    expect(languageFromPath("Common.hlsli")).toBe("hlsl");
    expect(languageFromPath("Bloom.compute")).toBe("hlsl");
    expect(languageFromPath("UnityCG.cginc")).toBe("hlsl");
    expect(languageFromPath("pass.usf")).toBe("hlsl");
    expect(languageFromPath("sky.vert")).toBe("cpp");
    expect(languageFromPath("sky.frag")).toBe("cpp");
    expect(languageFromPath("lib.glsl")).toBe("cpp");
    expect(languageFromPath("pbr.wgsl")).toBe("wgsl");
    expect(languageFromPath("Blit.metal")).toBe("cpp");
  });

  it("falls back to plaintext", () => {
    expect(languageFromPath("notes")).toBe("plaintext");
    expect(languageFromPath("a.bin")).toBe("plaintext");
  });
});
