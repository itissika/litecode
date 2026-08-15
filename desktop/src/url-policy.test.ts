import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { isAllowedExternalUrl, isAllowedLoadUrl } from "./url-policy";

describe("isAllowedExternalUrl (openExternal whitelist)", () => {
  it("allows http and https links", () => {
    assert.equal(isAllowedExternalUrl("https://example.com/doc"), true);
    assert.equal(isAllowedExternalUrl("http://127.0.0.1:7483/health"), true);
  });

  it("allows mailto links", () => {
    assert.equal(isAllowedExternalUrl("mailto:team@example.com"), true);
  });

  it("rejects non-whitelisted schemes", () => {
    assert.equal(isAllowedExternalUrl("file:///etc/passwd"), false);
    assert.equal(isAllowedExternalUrl("javascript:alert(1)"), false);
    assert.equal(isAllowedExternalUrl("data:text/html,<b>hi</b>"), false);
    assert.equal(isAllowedExternalUrl("custom://host/path"), false);
    assert.equal(isAllowedExternalUrl("smb://server/share"), false);
  });

  it("rejects unparseable input", () => {
    assert.equal(isAllowedExternalUrl(""), false);
    assert.equal(isAllowedExternalUrl("not a url"), false);
    assert.equal(isAllowedExternalUrl("http://"), false);
  });
});

describe("isAllowedLoadUrl (loadURL whitelist)", () => {
  it("allows http and https", () => {
    assert.equal(isAllowedLoadUrl("http://127.0.0.1:41723/?token=abc"), true);
    assert.equal(isAllowedLoadUrl("https://app.example.com/ws"), true);
  });

  it("rejects non-http(s) schemes including file", () => {
    assert.equal(isAllowedLoadUrl("file:///c:/windows/system32"), false);
    assert.equal(isAllowedLoadUrl("mailto:someone@example.com"), false);
    assert.equal(isAllowedLoadUrl("data:text/plain,hi"), false);
  });
});
