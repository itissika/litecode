import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  BROWSER_AGENT_ACTIVITY_CHANNEL,
  BROWSER_CLOSE_CHANNEL,
  BROWSER_OPEN_CHANNEL,
} from "./bridge-contract";
import { createBrowserAutomation } from "./browser-automation";
import {
  BrowserHost,
  type BrowserBounds,
  type BrowserPage,
  type BrowserViewHandle,
} from "./browser-host";
import { isAllowedLoadUrl } from "./url-policy";

const RECT = { x: 10, y: 40, width: 800, height: 600 };

function isMaskScript(code: string): boolean {
  return code.includes("__litecodeTimer");
}

class FakePage implements BrowserPage {
  url = "";
  title = "";
  destroyed = false;
  inputBlocked = false;
  inputBlockedAtCall = false;
  lastScript = "";
  scripts: string[] = [];
  nextResult: unknown = null;
  private listeners: Array<() => void> = [];

  loadURL(url: string): Promise<void> {
    this.url = url;
    this.title = "Example";
    for (const listener of this.listeners) listener();
    return Promise.resolve();
  }
  getURL(): string {
    return this.url;
  }
  getTitle(): string {
    return this.title;
  }
  goBack(): void {}
  goForward(): void {}
  reload(): void {}
  stop(): void {}
  canGoBack(): boolean {
    return false;
  }
  canGoForward(): boolean {
    return false;
  }
  executeJavaScript(code: string): Promise<unknown> {
    this.lastScript = code;
    this.scripts.push(code);
    this.inputBlockedAtCall = this.inputBlocked;
    return Promise.resolve(this.nextResult);
  }
  setInputBlocked(blocked: boolean): void {
    this.inputBlocked = blocked;
  }
  close(): void {
    this.destroyed = true;
  }
  isDestroyed(): boolean {
    return this.destroyed;
  }
  onNavigate(listener: () => void): void {
    this.listeners.push(listener);
  }
  onLoading(): void {}
  setWindowOpenHandler(): void {}
}

class FakeView implements BrowserViewHandle {
  readonly webContents = new FakePage();
  setBounds(): void {}
  setVisible(): void {}
  setBackgroundColor(): void {}
  attach(): void {}
  detach(): void {}
  reparent(): boolean {
    return true;
  }
}

function setup(
  options: {
    navigateOnOpen?: boolean;
    openReadyTimeoutMs?: number;
    busyLabel?: () => string;
  } = {},
) {
  const navigateOnOpen = options.navigateOnOpen ?? true;
  const views = new Map<string, FakeView>();
  const activity: boolean[] = [];
  let latestView: FakeView | null = null;
  const host = new BrowserHost({
    isAllowedUrl: isAllowedLoadUrl,
    createView: () => {
      latestView = new FakeView();
      return latestView;
    },
    onState: () => undefined,
  });
  const send = (channel: string, payload: unknown) => {
    if (channel === BROWSER_AGENT_ACTIVITY_CHANNEL) {
      activity.push((payload as { busy: boolean }).busy);
      return;
    }
    if (channel === BROWSER_OPEN_CHANNEL) {
      const { id, url } = payload as { id: string; url: string };
      host.create(id, "#0a0a0a");
      if (latestView) views.set(id, latestView);
      if (navigateOnOpen) {
        host.navigate(id, url);
        host.setVisible(id, true);
        host.setBounds(id, RECT);
        host.setActive(id, true);
      }
      return;
    }
    if (channel === BROWSER_CLOSE_CHANNEL) {
      host.destroy((payload as { id: string }).id);
    }
  };
  const automation = createBrowserAutomation({
    getHost: () => host,
    send,
    ...(options.openReadyTimeoutMs === undefined
      ? {}
      : { openReadyTimeoutMs: options.openReadyTimeoutMs }),
    ...(options.busyLabel === undefined ? {} : { busyLabel: options.busyLabel }),
  });
  return { host, automation, views, activity };
}

describe("browser automation", () => {
  it("opens a visible tab, lists it, reads it, and closes its own tab", async () => {
    const { automation, host, views, activity } = setup();
    const page = (await automation.open({ url: "https://example.com" })) as {
      id: string;
      visible: boolean;
    };
    assert.equal(page.visible, true);
    assert.equal(host.isAgentOwned(page.id), true);
    assert.deepEqual(automation.list(), {
      pages: [
        {
          id: page.id,
          url: "https://example.com",
          title: "Example",
          active: true,
          visible: true,
          agentOwned: true,
        },
      ],
      active_id: page.id,
    });

    const view = views.get(page.id);
    assert.ok(view);
    view.webContents.nextResult = {
      url: "https://example.com",
      title: "Example",
      text: "hello",
      truncated: false,
      elements: [{ ref: "e1", tag: "button", text: "Go" }],
    };
    assert.deepEqual(await automation.read({ page_id: page.id, max_chars: 1000 }), {
      page_id: page.id,
      url: "https://example.com",
      title: "Example",
      text: "hello",
      truncated: false,
      elements: [{ ref: "e1", tag: "button", text: "Go" }],
    });
    assert.doesNotThrow(() => new Function(view.webContents.lastScript));

    view.webContents.nextResult = { ok: true, tag: "button", text: "Go" };
    assert.deepEqual(await automation.click({ page_id: page.id, ref: "e1" }), {
      page_id: page.id,
      ref: "e1",
      tag: "button",
      text: "Go",
      url: "https://example.com",
      title: "Example",
    });
    assert.equal(view.webContents.inputBlockedAtCall, true);
    assert.equal(view.webContents.inputBlocked, false);
    assert.equal(view.webContents.scripts.filter(isMaskScript).length, 2);

    view.webContents.nextResult = { ok: true, tag: "input" };
    assert.deepEqual(await automation.type({ page_id: page.id, ref: "e1", text: "hi" }), {
      page_id: page.id,
      ref: "e1",
      tag: "input",
      url: "https://example.com",
      title: "Example",
    });
    assert.equal(view.webContents.inputBlockedAtCall, true);
    assert.doesNotThrow(() => new Function(view.webContents.lastScript));
    assert.equal(view.webContents.scripts.filter(isMaskScript).length, 4);
    for (const code of view.webContents.scripts) {
      assert.doesNotThrow(() => new Function(code));
    }
    assert.deepEqual(activity, [true, false, true, false]);

    assert.deepEqual(await automation.close({ page_id: page.id }), {
      id: page.id,
      closed: true,
    });
    assert.equal(host.pageInfo(page.id), null);
  });

  it("uses the localized busy label in the page mask", async () => {
    const label = "智能体正在操作此页面…";
    const { automation, views } = setup({ busyLabel: () => label });
    const page = (await automation.open({ url: "https://example.com" })) as {
      id: string;
    };
    const view = views.get(page.id);
    assert.ok(view);
    view.webContents.nextResult = { ok: true, tag: "button", text: "Go" };
    await automation.click({ page_id: page.id, ref: "e1" });
    assert.ok(view.webContents.scripts.some((code) => code.includes(label)));
  });

  it("fails an open that never loads and removes the blank tab", async () => {
    const { automation, host } = setup({
      navigateOnOpen: false,
      openReadyTimeoutMs: 50,
    });
    await assert.rejects(
      () => automation.open({ url: "https://example.com" }),
      (error: { code?: string }) => error.code === "timeout",
    );
    assert.deepEqual(host.list(), []);
  });

  it("refuses to close a page the agent did not open", async () => {
    const { automation, host } = setup();
    const humanId = "browser-00000000-0000-4000-8000-000000000000";
    host.create(humanId, "#0a0a0a");
    host.navigate(humanId, "https://example.com");
    await assert.rejects(
      () => automation.close({ page_id: humanId }),
      (error: { code?: string }) => error.code === "permission_denied",
    );
    assert.ok(host.pageInfo(humanId));
  });
});
