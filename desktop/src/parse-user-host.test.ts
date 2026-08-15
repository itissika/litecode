import assert from "node:assert/strict";
import test from "node:test";

import { formatUserAtHost, parseUserAtHost } from "./parse-user-host";

test("parseUserAtHost accepts user@host", () => {
  assert.deepEqual(parseUserAtHost("alice@box.example"), {
    user: "alice",
    host: "box.example",
  });
});

test("parseUserAtHost accepts user@host:port", () => {
  assert.deepEqual(parseUserAtHost("alice@box.example:2222"), {
    user: "alice",
    host: "box.example",
    port: 2222,
  });
});

test("formatUserAtHost roundtrips label", () => {
  assert.equal(formatUserAtHost({ user: "alice", host: "10.0.0.2", port: 22 }), "alice@10.0.0.2:22");
});
