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

  it("keeps rules", () => {
    const result = parseCustomToolJson(`{
      "name": "echo_py",
      "command": "python",
      "schema": { "type": "object", "properties": {}, "required": [] },
      "rules": [
        {
          "id": "outside_workspace",
          "when": { "kind": "path_outside_workspace", "name": "path" },
          "action": "deny"
        }
      ]
    }`);
    expect(result).toMatchObject({
      ok: {
        rules: [
          {
            id: "outside_workspace",
            action: "deny",
            when: { kind: "path_outside_workspace", name: "path" },
          },
        ],
      },
    });
  });

  it("treats an empty rules array as no rules", () => {
    const result = parseCustomToolJson(`{
      "name": "echo_py",
      "command": "python",
      "schema": { "type": "object", "properties": {}, "required": [] },
      "rules": []
    }`);
    expect(result).toMatchObject({ ok: { name: "echo_py" } });
    if (!("ok" in result)) throw new Error("expected ok");
    expect(result.ok.rules).toBeUndefined();
  });

  it("trims rule ids and accepts nested matchers", () => {
    const result = parseCustomToolJson(`{
      "name": "echo_py",
      "command": "python",
      "schema": { "type": "object", "properties": {}, "required": [] },
      "rules": [{
        "id": "  nested  ",
        "action": "deny",
        "when": {
          "kind": "all_of",
          "matchers": [{ "kind": "any" }, { "kind": "any_of", "matchers": [{ "kind": "bash_readonly_command" }] }]
        }
      }]
    }`);
    expect(result).toMatchObject({
      ok: { rules: [{ id: "nested", action: "deny" }] },
    });
  });

  it("rejects blank, reserved, duplicate, and unknown rule matchers", () => {
    const tool = (rules: string) => `{
      "name": "echo_py",
      "command": "python",
      "schema": { "type": "object", "properties": {}, "required": [] },
      "rules": [${rules}]
    }`;
    expect(parseCustomToolJson(tool(`{ "id": "  ", "action": "deny", "when": { "kind": "any" } }`))).toEqual({
      skip: "invalid",
    });
    expect(
      parseCustomToolJson(tool(`{ "id": "__default", "action": "deny", "when": { "kind": "any" } }`)),
    ).toEqual({ skip: "invalid" });
    expect(
      parseCustomToolJson(
        tool(
          `{ "id": "a", "action": "deny", "when": { "kind": "any" } }, { "id": " a ", "action": "allow", "when": { "kind": "any" } }`,
        ),
      ),
    ).toEqual({ skip: "invalid" });
    expect(
      parseCustomToolJson(tool(`{ "id": "a", "action": "deny", "when": { "kind": "nope" } }`)),
    ).toEqual({ skip: "invalid" });
    expect(
      parseCustomToolJson(tool(`{ "id": "a", "action": "deny", "when": { "kind": "all_of" } }`)),
    ).toEqual({ skip: "invalid" });
    expect(
      parseCustomToolJson(
        tool(`{ "id": "a", "action": "deny", "when": { "kind": "any_of", "matchers": [] } }`),
      ),
    ).toEqual({ skip: "invalid" });
  });

  it("rejects a rule without an action", () => {
    expect(
      parseCustomToolJson(`{
        "name": "echo_py",
        "command": "python",
        "schema": { "type": "object", "properties": {}, "required": [] },
        "rules": [{ "id": "x", "when": { "kind": "any" } }]
      }`),
    ).toEqual({ skip: "invalid" });
  });

  it("rejects name changes on an existing tool", () => {
    expect(
      parseCustomToolJson(
        `{"name":"other","command":"x","schema":{"type":"object","properties":{},"required":[]}}`,
        "echo_py",
      ),
    ).toEqual({
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
      ok: { id: "filesystem", def: { command: "npx", timeout: 60 } },
    });
  });

  it("keeps a declared timeout", () => {
    const result = parseMcpJson(`{
      "id": "filesystem",
      "command": "npx",
      "timeout": 300,
      "transport": { "type": "stdio" }
    }`);
    expect(result).toMatchObject({
      ok: { def: { timeout: 300 } },
    });
  });

  it("rejects missing stdio command", () => {
    expect(
      parseMcpJson(`{"id":"x","command":"","transport":{"type":"stdio"}}`),
    ).toEqual({
      skip: "invalid",
    });
  });
});
