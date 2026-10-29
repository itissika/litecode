import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { isAllowedExternalUrl, isAllowedLoadUrl, sameAppHttpOrigin } from "./url-policy";

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

describe("sameAppHttpOrigin (local app navigation)", () => {
  it("treats equivalent loopback spellings on one port as the same app", () => {
    assert.equal(sameAppHttpOrigin("http://127.0.0.1:5123/a", "http://localhost:5123/b"), true);
    assert.equal(sameAppHttpOrigin("http://[::1]:5123/", "http://127.0.0.1:5123/"), true);
  });

  it("keeps other ports, hosts, and schemes outside the app", () => {
    assert.equal(sameAppHttpOrigin("http://127.0.0.1:5124/", "http://127.0.0.1:5123/"), false);
    assert.equal(sameAppHttpOrigin("https://127.0.0.1:5123/", "http://127.0.0.1:5123/"), false);
    assert.equal(sameAppHttpOrigin("http://example.com:5123/", "http://127.0.0.1:5123/"), false);
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
