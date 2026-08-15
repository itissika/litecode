import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { handshakeUrl } from "./handshake-url";

describe("handshakeUrl", () => {
  it("appends token query and trailing slash base", () => {
    assert.equal(
      handshakeUrl("http://127.0.0.1:7483", "abc"),
      "http://127.0.0.1:7483/?token=abc",
    );
  });

  it("preserves existing path", () => {
    assert.equal(
      handshakeUrl("https://box.example:7483/", "tok"),
      "https://box.example:7483/?token=tok",
    );
  });
});
