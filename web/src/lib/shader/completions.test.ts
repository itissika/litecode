import { describe, expect, it } from "vitest";

import { shaderSuggests } from "./completions";

describe("shaderSuggests", () => {
  const labels = () => shaderSuggests().map((item) => item.label);

  it("includes HLSL intrinsics and types", () => {
    expect(labels()).toEqual(
      expect.arrayContaining(["lerp", "saturate", "float4"]),
    );
  });

  it("includes ShaderLab and Unity built-ins", () => {
    expect(labels()).toEqual(
      expect.arrayContaining(["HLSLPROGRAM", "_Time", "UnityObjectToClipPos"]),
    );
  });

  it("includes URP helpers", () => {
    expect(labels()).toEqual(
      expect.arrayContaining([
        "TransformObjectToHClip",
        "GetMainLight",
        "SAMPLE_TEXTURE2D",
      ]),
    );
  });
});
