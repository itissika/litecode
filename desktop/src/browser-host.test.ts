import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  BrowserHost,
  type BrowserBounds,
  type BrowserPage,
  type BrowserViewHandle,
} from "./browser-host";
import { isAllowedLoadUrl } from "./url-policy";

const PANEL_ID = "browser-11111111-1111-4111-8111-111111111111";

class FakePage implements BrowserPage {
  url = "";
  title = "";
  back = false;
  forward = false;
  destroyed = false;
  loads: string[] = [];
  closed = 0;
  private navigateListeners: Array<() => void> = [];
  private loadingListeners: Array<(loading: boolean) => void> = [];
  private openHandler: ((url: string) => void) | null = null;

  loadURL(url: string): Promise<void> {
    this.loads.push(url);
    this.url = url;
    this.title = "Example";
    this.back = true;
    for (const listener of this.navigateListeners) listener();
    return Promise.resolve();
  }

  getURL(): string {
    return this.url;
  }

  getTitle(): string {
    return this.title;
  }

  goBack(): void {
    this.back = false;
    this.forward = true;
  }

  goForward(): void {
    this.forward = false;
    this.back = true;
  }

  canGoBack(): boolean {
    return this.back;
  }

  canGoForward(): boolean {
    return this.forward;
  }

  close(): void {
    this.closed += 1;
    this.destroyed = true;
  }

  isDestroyed(): boolean {
    return this.destroyed;
  }

  onNavigate(listener: () => void): void {
    this.navigateListeners.push(listener);
  }

  onLoading(listener: (loading: boolean) => void): void {
    this.loadingListeners.push(listener);
  }

  setWindowOpenHandler(handler: (url: string) => void): void {
    this.openHandler = handler;
  }

  open(url: string): void {
    this.openHandler?.(url);
  }
}

class FakeView implements BrowserViewHandle {
  readonly webContents = new FakePage();
  bounds: BrowserBounds[] = [];
  visible: boolean[] = [];
  color = "";
  attached = 0;
  detached = 0;

  setBounds(bounds: BrowserBounds): void {
    this.bounds.push(bounds);
  }

  setVisible(visible: boolean): void {
    this.visible.push(visible);
  }

  setBackgroundColor(color: string): void {
    this.color = color;
  }

  attach(): void {
    this.attached += 1;
  }

  detach(): void {
    this.detached += 1;
  }

  reparent(): boolean {
    return true;
  }
}

function hostWith(view: FakeView): BrowserHost {
  return new BrowserHost({
    isAllowedUrl: isAllowedLoadUrl,
    createView: () => view,
    onState: () => undefined,
  });
}

const RECT = { x: 10, y: 40, width: 800, height: 600 };

describe("BrowserHost", () => {
  it("rejects a non-http url and an invalid panel id", () => {
    const view = new FakeView();
    const host = hostWith(view);
    host.create(PANEL_ID, "#111111");
    assert.throws(() => host.navigate(PANEL_ID, "file:///etc/passwd"), /http/);
    assert.equal(view.webContents.loads.length, 0);
    assert.throws(() => host.create("../escape", "#111111"), /Invalid browser panel id/);
  });

  it("creates one view for a repeated id and opens https", () => {
    const view = new FakeView();
    let created = 0;
    const host = new BrowserHost({
      isAllowedUrl: isAllowedLoadUrl,
      createView: () => {
        created += 1;
        return view;
      },
      onState: () => undefined,
    });
    host.create(PANEL_ID, "rgb(10, 10, 10)");
    host.create(PANEL_ID, "rgb(10, 10, 10)");
    assert.equal(created, 1);
    assert.equal(view.attached, 1);
    assert.equal(view.color, "rgb(10, 10, 10)");
    const state = host.navigate(PANEL_ID, "https://example.com/docs");
    assert.deepEqual(view.webContents.loads, ["https://example.com/docs"]);
    assert.equal(state.url, "https://example.com/docs");
    assert.equal(state.canGoBack, true);
    assert.equal(state.loading, true);
  });

  it("falls back when the background color is not a color", () => {
    const view = new FakeView();
    const host = hostWith(view);
    host.create(PANEL_ID, "javascript:alert(1)");
    assert.equal(view.color, "#0a0a0a");
  });

  it("does not place a rectangle while obscured", () => {
    const view = new FakeView();
    const host = hostWith(view);
    host.setObscured(true);
    host.create(PANEL_ID, "#0a0a0a");
    view.visible = [];
    view.bounds = [];
    host.setBounds(PANEL_ID, RECT);
    host.setVisible(PANEL_ID, true);
    assert.deepEqual(view.bounds, []);
    assert.deepEqual(view.visible, []);

    host.setObscured(false);
    assert.deepEqual(view.bounds, [RECT]);
    assert.deepEqual(view.visible, [true]);

    view.bounds = [];
    host.setObscured(true);
    host.setBounds(PANEL_ID, { x: 1, y: 2, width: 30, height: 40 });
    assert.deepEqual(view.bounds, []);
    assert.equal(view.visible.at(-1), false);
  });

  it("hides a zero-area rectangle and destroys once", () => {
    const view = new FakeView();
    const host = hostWith(view);
    host.create(PANEL_ID, "#0a0a0a");
    host.setBounds(PANEL_ID, RECT);
    host.setVisible(PANEL_ID, true);
    view.bounds = [];
    host.setBounds(PANEL_ID, { x: 0, y: 0, width: 0, height: 0 });
    assert.deepEqual(view.bounds, []);
    assert.equal(view.visible.at(-1), false);

    host.destroy(PANEL_ID);
    host.destroy(PANEL_ID);
    assert.equal(view.detached, 1);
    assert.equal(view.webContents.closed, 1);
    host.setBounds(PANEL_ID, RECT);
    assert.deepEqual(view.bounds, []);
  });

  it("opens an http popup in the same page and ignores other schemes", () => {
    const view = new FakeView();
    const host = hostWith(view);
    host.create(PANEL_ID, "#0a0a0a");
    view.webContents.open("https://example.com/popup");
    assert.deepEqual(view.webContents.loads, ["https://example.com/popup"]);
    view.webContents.open("file:///c:/windows");
    assert.deepEqual(view.webContents.loads, ["https://example.com/popup"]);
  });

  it("moves history only when that direction is available", () => {
    const view = new FakeView();
    const host = hostWith(view);
    host.create(PANEL_ID, "#0a0a0a");
    host.goBack(PANEL_ID);
    assert.equal(view.webContents.forward, false);
    view.webContents.back = true;
    host.goBack(PANEL_ID);
    assert.equal(view.webContents.forward, true);
    host.goForward(PANEL_ID);
    assert.equal(view.webContents.back, true);
  });

  it("applies a rectangle only for the window the view is hanging on", () => {
    const view = new FakeView();
    const moves: Array<string | null> = [];
    const host = new BrowserHost({
      isAllowedUrl: isAllowedLoadUrl,
      createView: () => view,
      onState: () => undefined,
      moveView: (_view, popoutId) => {
        moves.push(popoutId);
        return popoutId !== "bbbbbbbb-bbbb-4ccc-8ddd-eeeeeeeeeeee";
      },
    });
    const dock = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    const missing = "bbbbbbbb-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    host.create(PANEL_ID, "#0a0a0a");
    host.setVisible(PANEL_ID, true);
    host.setBounds(PANEL_ID, RECT);
    assert.deepEqual(view.bounds, [RECT]);

    view.bounds = [];
    assert.equal(host.setHost(PANEL_ID, dock), true);
    assert.deepEqual(moves, [dock]);
    host.setBounds(PANEL_ID, { x: 8, y: 9, width: 20, height: 30 }, "main");
    assert.deepEqual(view.bounds, []);
    host.setBounds(PANEL_ID, { x: 8, y: 9, width: 20, height: 30 }, dock);
    assert.deepEqual(view.bounds, [{ x: 8, y: 9, width: 20, height: 30 }]);

    view.bounds = [];
    view.visible = [];
    assert.equal(host.setHost(PANEL_ID, missing), false);
    assert.deepEqual(moves, [dock, missing, null]);
    assert.equal(view.visible.at(-1), false);
    host.setBounds(PANEL_ID, RECT, dock);
    assert.deepEqual(view.bounds, []);
    host.setVisible(PANEL_ID, true);
    host.setBounds(PANEL_ID, RECT, "main");
    assert.deepEqual(view.bounds, [RECT]);
  });

  it("parks a popout view back on the main window without a rectangle", () => {
    const view = new FakeView();
    const moves: Array<string | null> = [];
    const host = new BrowserHost({
      isAllowedUrl: isAllowedLoadUrl,
      createView: () => view,
      onState: () => undefined,
      moveView: (_view, popoutId) => {
        moves.push(popoutId);
        return true;
      },
    });
    const dock = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    host.create(PANEL_ID, "#0a0a0a");
    host.setHost(PANEL_ID, dock);
    host.setVisible(PANEL_ID, true);
    host.setBounds(PANEL_ID, RECT, dock);
    view.bounds = [];
    view.visible = [];
    host.parkPopout(dock);
    assert.deepEqual(moves, [dock, null]);
    assert.equal(view.visible.at(-1), false);
    host.setBounds(PANEL_ID, RECT, dock);
    assert.deepEqual(view.bounds, []);
    host.setVisible(PANEL_ID, true);
    host.setBounds(PANEL_ID, RECT, "main");
    assert.deepEqual(view.bounds, [RECT]);
  });
});
