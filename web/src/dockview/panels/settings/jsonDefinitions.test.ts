import { describe, expect, it } from "vitest";

import { parseCustomToolJson, parseMcpJson } from "./jsonDefinitions";

describe("parseCustomToolJson", () => {
  it("accepts a complete definition", () => {
    const result = parseCustomToolJson(`{
      "name": "echo_py",
      "command": "python",
      "args": ["echo.py"],
      "schema": { "type": "object", "properties": {}, "required": [] }
    }`);
    expect(result).toMatchObject({
      ok: { name: "echo_py", command: "python", args: ["echo.py"] },
    });
  });

  it("rejects name changes on an existing tool", () => {
    expect(parseCustomToolJson(`{"name":"other","command":"x","schema":{"type":"object","properties":{},"required":[]}}`, "echo_py")).toEqual({
      skip: "invalid",
    });
  });
});

describe("parseMcpJson", () => {
  it("accepts a stdio server", () => {
    const result = parseMcpJson(`{
      "id": "filesystem",
      "command": "npx",
      "args": ["-y", "server"],
      "transport": { "type": "stdio" }
    }`);
    expect(result).toMatchObject({
      ok: { id: "filesystem", def: { command: "npx" } },
    });
  });

  it("rejects missing stdio command", () => {
    expect(parseMcpJson(`{"id":"x","command":"","transport":{"type":"stdio"}}`)).toEqual({
      skip: "invalid",
    });
  });
});
