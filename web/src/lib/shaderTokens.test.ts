import { describe, expect, it } from "vitest";

import { INITIAL } from "shiki/textmate";

import { getMarkdownHighlighter } from "./shiki";

describe("shader shiki grammars", () => {
  it("tokenizes HLSL keywords", async () => {
    const highlighter = await getMarkdownHighlighter();
    const grammar = highlighter.getLanguage("hlsl");
    const result = grammar.tokenizeLine(
      "float4 color = lerp(a, b, t);",
      INITIAL,
    );
    const scopes = result.tokens.flatMap((token) => token.scopes);
    expect(
      scopes.some(
        (scope) => scope.includes("storage") || scope.includes("keyword"),
      ),
    ).toBe(true);
    expect(scopes.some((scope) => scope.includes("hlsl"))).toBe(true);
  });

  it("tokenizes ShaderLab wrappers", async () => {
    const highlighter = await getMarkdownHighlighter();
    const grammar = highlighter.getLanguage("shaderlab");
    const result = grammar.tokenizeLine('Shader "Unlit/Color" {', INITIAL);
    const scopes = result.tokens.flatMap((token) => token.scopes);
    expect(scopes.some((scope) => scope.includes("shaderlab"))).toBe(true);
  });
});
