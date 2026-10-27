import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  BrowserBridgeError,
  startBrowserBridge,
  type BrowserBridgeHandlers,
} from "./browser-bridge";

function handlers(overrides: Partial<BrowserBridgeHandlers> = {}): BrowserBridgeHandlers {
  return {
    open: async (input) => ({ id: "browser-1", url: input.url }),
    close: async (input) => ({ id: input.page_id, closed: true }),
    list: () => ({ pages: [], active_id: null }),
    read: async () => ({ page_id: "browser-1", text: "hello" }),
    click: async () => ({ page_id: "browser-1", ref: "e1" }),
    type: async () => ({ page_id: "browser-1", ref: "e1" }),
    ...overrides,
  };
}

async function withBridge(
  run: (url: string, token: string) => Promise<void>,
  overrides: Partial<BrowserBridgeHandlers> = {},
): Promise<void> {
  const bridge = await startBrowserBridge(handlers(overrides), "test-token");
  try {
    await run(bridge.url, bridge.token);
  } finally {
    await bridge.close();
  }
}

function auth(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}` };
}

describe("browser bridge", () => {
  it("rejects requests without the bearer token", async () => {
    await withBridge(async (url) => {
      const response = await fetch(`${url}/list`);
      assert.equal(response.status, 401);
      assert.deepEqual(await response.json(), {
        ok: false,
        code: "unauthorized",
        message: "Browser bridge authorization failed",
      });
    });
  });

  it("forwards list and open requests", async () => {
    await withBridge(async (url, token) => {
      const listed = await fetch(`${url}/list`, { headers: auth(token) });
      assert.equal(listed.status, 200);
      assert.deepEqual(await listed.json(), {
        ok: true,
        data: { pages: [], active_id: null },
      });

      const opened = await fetch(`${url}/open`, {
        method: "POST",
        headers: { ...auth(token), "content-type": "application/json" },
        body: JSON.stringify({ url: "https://example.com" }),
      });
      assert.equal(opened.status, 200);
      assert.deepEqual(await opened.json(), {
        ok: true,
        data: { id: "browser-1", url: "https://example.com" },
      });
    });
  });

  it("maps domain errors and rejects malformed bodies", async () => {
    await withBridge(
      async (url, token) => {
        const denied = await fetch(`${url}/click`, {
          method: "POST",
          headers: { ...auth(token), "content-type": "application/json" },
          body: JSON.stringify({ ref: "e1" }),
        });
        assert.equal(denied.status, 403);
        assert.deepEqual(await denied.json(), {
          ok: false,
          code: "permission_denied",
          message: "Only agent tabs",
        });

        const malformed = await fetch(`${url}/open`, {
          method: "POST",
          headers: { ...auth(token), "content-type": "application/json" },
          body: "{",
        });
        assert.equal(malformed.status, 400);
        assert.deepEqual(await malformed.json(), {
          ok: false,
          code: "invalid_input",
          message: "Request body must be JSON",
        });

        const missing = await fetch(`${url}/nope`, {
          method: "POST",
          headers: auth(token),
        });
        assert.equal(missing.status, 404);
      },
      {
        click: async () => {
          throw new BrowserBridgeError("permission_denied", "Only agent tabs", 403);
        },
      },
    );
  });
});
