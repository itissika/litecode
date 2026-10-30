/**
 * Owns embedded browser pages for the workbench. Electron view creation stays
 * in `browser-view.ts`; this module only tracks identity, bounds, and navigation
 * so it can be tested without a window.
 */

export const BROWSER_PARTITION = "persist:litecode-browser";

const BROWSER_ID_PATTERN =
  /^browser-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const DEFAULT_BACKGROUND = "#0a0a0a";
const MAX_URL_LENGTH = 8000;
const POPOUT_DOCK_ID =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export type BrowserBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

export type BrowserState = {
  id: string;
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
};

/** One open page as the agent sees it. */
export type BrowserPageInfo = {
  id: string;
  url: string;
  title: string;
  active: boolean;
  visible: boolean;
  agentOwned: boolean;
};

export interface BrowserPage {
  loadURL(url: string): Promise<void>;
  getURL(): string;
  getTitle(): string;
  goBack(): void;
  goForward(): void;
  reload(): void;
  stop(): void;
  canGoBack(): boolean;
  canGoForward(): boolean;
  /** Run page-context JavaScript. Rejects when the page is gone. */
  executeJavaScript(code: string): Promise<unknown>;
  /** Block page keyboard/mouse input without hiding the page. */
  setInputBlocked(blocked: boolean): void;
  /** JPEG data URL used as the still frame while the native view is hidden. */
  captureSnapshot(): Promise<string>;
  close(): void;
  isDestroyed(): boolean;
  onNavigate(listener: () => void): void;
  onLoading(listener: (loading: boolean) => void): void;
  /** Called with the target URL. The page itself must not open a new window. */
  setWindowOpenHandler(handler: (url: string) => void): void;
}

export interface BrowserViewHandle {
  readonly webContents: BrowserPage;
  setBounds(bounds: BrowserBounds): void;
  setVisible(visible: boolean): void;
  setBackgroundColor(color: string): void;
  attach(): void;
  detach(): void;
  /** Move the view onto another window. False leaves the caller to hide it. */
  reparent(target: unknown): boolean;
}

export interface BrowserHostOptions {
  isAllowedUrl: (url: string, id: string) => boolean;
  createView: (id: string) => BrowserViewHandle;
  onState: (state: BrowserState) => void;
  /**
   * Move a view onto the main window (`null`) or a registered popout.
   * Absent means the view stays on the window it was created on.
   * False means the move did not happen.
   */
  moveView?: (view: BrowserViewHandle, popoutId: string | null) => boolean;
  /** Title used while a page has no title of its own. */
  fallbackTitle?: string;
  /** Restrict ids accepted by this host. Defaults to browser panel ids. */
  idPattern?: RegExp;
  /** How often visible pages refresh their still frame. 0 disables the timer. */
  snapshotIntervalMs?: number;
  /** Replaces the default "load in this page" window-open behavior. */
  onWindowOpen?: (id: string, url: string) => void;
}

type Entry = {
  view: BrowserViewHandle;
  wantsVisible: boolean;
  shown: boolean;
  bounds: BrowserBounds | null;
  loading: boolean;
  /** Hidden by an overlapping workbench surface, not by the page itself. */
  occluded: boolean;
  /** Last JPEG data URL captured while the page was visible. */
  lastSnapshot: string;
  snapshotRequest: Promise<string> | null;
  /** `"main"` or the popout dock id the view is attached to. */
  place: string;
};

function normalizeBrowserBackground(color: unknown): string {
  if (typeof color !== "string") return DEFAULT_BACKGROUND;
  const trimmed = color.trim();
  if (trimmed.length === 0 || trimmed.length > 80) return DEFAULT_BACKGROUND;
  if (
    /^#[0-9a-fA-F]{3,8}$/.test(trimmed) ||
    /^(rgb|hsl)a?\([^)]+\)$/.test(trimmed)
  ) {
    return trimmed;
  }
  return DEFAULT_BACKGROUND;
}

function sanitizeBrowserBounds(value: unknown): BrowserBounds | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  const x = Number(raw.x);
  const y = Number(raw.y);
  const width = Number(raw.width);
  const height = Number(raw.height);
  if (![x, y, width, height].every(Number.isFinite)) return null;
  const w = Math.round(width);
  const h = Math.round(height);
  if (w <= 0 || h <= 0 || w > 10000 || h > 10000) return null;
  return { x: Math.round(x), y: Math.round(y), width: w, height: h };
}

function isZeroArea(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  const raw = value as Record<string, unknown>;
  const width = Number(raw.width);
  const height = Number(raw.height);
  return (
    Number.isFinite(width) &&
    Number.isFinite(height) &&
    (width <= 0 || height <= 0)
  );
}

export class BrowserHost {
  private readonly pages = new Map<string, Entry>();
  private readonly idPattern: RegExp;
  private readonly agentOwned = new Set<string>();
  private activePanelId: string | null = null;
  private obscured = false;
  private snapshotInFlight = false;
  private snapshotTimer: ReturnType<typeof setInterval> | null = null;

  constructor(private readonly options: BrowserHostOptions) {
    this.idPattern = options.idPattern ?? BROWSER_ID_PATTERN;
    const interval = options.snapshotIntervalMs ?? 0;
    if (interval > 0) {
      const timer = setInterval(() => {
        void this.captureVisibleSnapshots();
      }, interval);
      if (typeof timer === "object" && "unref" in timer) timer.unref();
      this.snapshotTimer = timer;
    }
  }

  create(id: string, backgroundColor: unknown): BrowserState {
    const panelId = this.requireId(id);
    const existing = this.pages.get(panelId);
    if (existing) return this.snapshot(panelId, existing);

    const view = this.options.createView(panelId);
    view.setBackgroundColor(normalizeBrowserBackground(backgroundColor));
    view.setVisible(false);
    const entry: Entry = {
      view,
      wantsVisible: false,
      shown: false,
      bounds: null,
      loading: false,
      occluded: false,
      lastSnapshot: "",
      snapshotRequest: null,
      place: "main",
    };
    this.pages.set(panelId, entry);
    this.bind(panelId, entry);
    view.attach();
    return this.snapshot(panelId, entry);
  }

  /** The renderer tells us which visible panel is active. */
  setActive(id: unknown, active: boolean): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    if (active) {
      if (this.pages.has(panelId)) this.activePanelId = panelId;
      return;
    }
    if (this.activePanelId === panelId) this.activePanelId = null;
  }

  activeId(): string | null {
    if (!this.activePanelId) return null;
    const entry = this.pages.get(this.activePanelId);
    if (!entry || entry.view.webContents.isDestroyed()) {
      this.activePanelId = null;
      return null;
    }
    return this.activePanelId;
  }

  /** Main marks an id before the renderer has created its panel. */
  markAgentOwned(id: unknown): void {
    const panelId = this.readId(id);
    if (panelId) this.agentOwned.add(panelId);
  }

  isAgentOwned(id: unknown): boolean {
    const panelId = this.readId(id);
    return panelId !== null && this.agentOwned.has(panelId);
  }

  /** Drop ownership for an open request that never became a page. */
  clearAgentOwned(id: unknown): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    this.agentOwned.delete(panelId);
    if (this.activePanelId === panelId && !this.pages.has(panelId)) this.activePanelId = null;
  }

  list(): BrowserPageInfo[] {
    return [...this.pages.keys()]
      .map((id) => this.pageInfo(id))
      .filter((page): page is BrowserPageInfo => page !== null);
  }

  pageInfo(id: unknown): BrowserPageInfo | null {
    const panelId = this.readId(id);
    if (!panelId) return null;
    const entry = this.pages.get(panelId);
    if (!entry) return null;
    const page = entry.view.webContents;
    if (page.isDestroyed()) return null;
    const state = this.snapshot(panelId, entry);
    return {
      id: panelId,
      url: state.url,
      title: state.title,
      active: this.activeId() === panelId,
      visible: entry.shown,
      agentOwned: this.agentOwned.has(panelId),
    };
  }

  isShown(id: unknown): boolean {
    return this.pageInfo(id)?.visible === true;
  }

  executeJavaScript(id: unknown, code: string): Promise<unknown> {
    const panelId = this.readId(id);
    if (!panelId) return Promise.reject(new Error("Invalid browser panel id"));
    const entry = this.pages.get(panelId);
    if (!entry || entry.view.webContents.isDestroyed()) {
      return Promise.reject(new Error("Browser page is not open"));
    }
    return entry.view.webContents.executeJavaScript(code);
  }

  setInputBlocked(id: unknown, blocked: boolean): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    const entry = this.pages.get(panelId);
    if (!entry || entry.view.webContents.isDestroyed()) return;
    entry.view.webContents.setInputBlocked(blocked);
  }

  navigate(id: string, url: unknown): BrowserState {
    const panelId = this.requireId(id);
    const entry = this.requirePage(panelId);
    this.startLoad(panelId, entry, url);
    return this.snapshot(panelId, entry);
  }

  goBack(id: string): BrowserState {
    return this.go(id, "back");
  }

  goForward(id: string): BrowserState {
    return this.go(id, "forward");
  }

  /** Reload in place. The address bar and history stay where they are. */
  reload(id: string): BrowserState {
    const panelId = this.requireId(id);
    const entry = this.requirePage(panelId);
    entry.loading = true;
    entry.view.webContents.reload();
    this.emit(panelId, entry);
    return this.snapshot(panelId, entry);
  }

  /** Drop the pending load; whatever already rendered stays on screen. */
  stop(id: string): BrowserState {
    const panelId = this.requireId(id);
    const entry = this.requirePage(panelId);
    entry.view.webContents.stop();
    entry.loading = false;
    this.emit(panelId, entry);
    return this.snapshot(panelId, entry);
  }

  setBounds(id: unknown, bounds: unknown, place?: unknown): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    const entry = this.pages.get(panelId);
    if (!entry) return;
    if (isZeroArea(bounds)) {
      entry.bounds = null;
      this.apply(entry);
      return;
    }
    const claimed = typeof place === "string" && place.length > 0 ? place : "main";
    if (claimed !== entry.place) return;
    const next = sanitizeBrowserBounds(bounds);
    if (!next) return;
    entry.bounds = next;
    this.apply(entry);
  }

  setVisible(id: unknown, visible: boolean): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    const entry = this.pages.get(panelId);
    if (!entry) return;
    entry.wantsVisible = visible;
    this.apply(entry);
  }

  setObscured(obscured: boolean): void {
    this.obscured = obscured;
    for (const entry of this.pages.values()) this.apply(entry);
  }

  /** Hide one page while a workbench surface overlaps its rectangle. */
  setViewOccluded(id: unknown, occluded: boolean): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    const entry = this.pages.get(panelId);
    if (!entry || entry.occluded === occluded) return;
    entry.occluded = occluded;
    this.apply(entry);
  }

  /** Latest still frame, captured on demand when the cache is still empty. */
  getSnapshot(id: unknown): Promise<string> {
    const panelId = this.readId(id);
    if (!panelId) return Promise.resolve("");
    const entry = this.pages.get(panelId);
    if (!entry || entry.view.webContents.isDestroyed()) return Promise.resolve("");
    if (entry.lastSnapshot) return Promise.resolve(entry.lastSnapshot);
    return this.captureSnapshot(panelId, entry);
  }

  dispose(): void {
    if (this.snapshotTimer) clearInterval(this.snapshotTimer);
    this.snapshotTimer = null;
  }

  /**
   * Hang the view on a popout window, or back on the main window when
   * `popoutId` is null. A failed popout hides the view instead of leaving a
   * rectangle on the main window.
   */
  setHost(id: unknown, popoutId: unknown): boolean {
    const panelId = this.readId(id);
    if (!panelId) return false;
    const entry = this.pages.get(panelId);
    if (!entry) return false;
    if (popoutId == null || popoutId === "") {
      return this.placeOnMain(entry);
    }
    if (typeof popoutId !== "string" || !POPOUT_DOCK_ID.test(popoutId)) {
      this.hide(entry);
      return false;
    }
    entry.bounds = null;
    this.apply(entry);
    const moved = this.options.moveView?.(entry.view, popoutId) ?? false;
    if (!moved) {
      this.placeOnMain(entry);
      this.hide(entry);
      return false;
    }
    entry.place = popoutId;
    this.apply(entry);
    return true;
  }

  /** A popout window is going away. Park its views on the main window, hidden. */
  parkPopout(popoutId: string): void {
    for (const entry of this.pages.values()) {
      if (entry.place !== popoutId) continue;
      this.hide(entry);
      this.placeOnMain(entry);
      this.hide(entry);
    }
  }

  destroy(id: unknown): void {
    const panelId = this.readId(id);
    if (!panelId) return;
    const entry = this.pages.get(panelId);
    if (!entry) return;
    this.pages.delete(panelId);
    this.agentOwned.delete(panelId);
    if (this.activePanelId === panelId) this.activePanelId = null;
    entry.shown = false;
    entry.lastSnapshot = "";
    entry.snapshotRequest = null;
    entry.view.webContents.setInputBlocked(false);
    entry.view.detach();
    entry.view.webContents.close();
  }

  destroyAll(): void {
    for (const id of [...this.pages.keys()]) this.destroy(id);
  }

  private go(id: string, direction: "back" | "forward"): BrowserState {
    const panelId = this.requireId(id);
    const entry = this.requirePage(panelId);
    const page = entry.view.webContents;
    if (direction === "back") {
      if (page.canGoBack()) page.goBack();
    } else if (page.canGoForward()) {
      page.goForward();
    }
    return this.snapshot(panelId, entry);
  }

  private startLoad(id: string, entry: Entry, url: unknown): void {
    if (typeof url !== "string" || url.length === 0 || url.length > MAX_URL_LENGTH) {
      throw new Error("Only http and https URLs can be opened");
    }
    if (!this.options.isAllowedUrl(url, id)) {
      throw new Error("Only http and https URLs can be opened");
    }
    entry.loading = true;
    void entry.view.webContents.loadURL(url).catch(() => {
      const current = this.pages.get(id);
      if (!current) return;
      current.loading = false;
      this.emit(id, current);
    });
    this.emit(id, entry);
  }

  private bind(id: string, entry: Entry): void {
    const page = entry.view.webContents;
    page.onLoading((loading) => {
      const current = this.pages.get(id);
      if (!current) return;
      current.loading = loading;
      this.emit(id, current);
    });
    page.onNavigate(() => {
      const current = this.pages.get(id);
      if (!current) return;
      this.emit(id, current);
    });
    page.setWindowOpenHandler((url) => {
      const current = this.pages.get(id);
      if (!current) return;
      if (this.options.onWindowOpen) {
        this.options.onWindowOpen(id, url);
        return;
      }
      try {
        this.startLoad(id, current, url);
      } catch {
        // Popups that are not http(s) stay on the current page.
      }
    });
  }

  private hide(entry: Entry): void {
    entry.bounds = null;
    this.apply(entry);
  }

  private placeOnMain(entry: Entry): boolean {
    const moved = this.options.moveView
      ? this.options.moveView(entry.view, null)
      : true;
    entry.place = "main";
    if (!moved) this.hide(entry);
    else this.apply(entry);
    return moved;
  }

  /** Show only a visible, unobscured page that has a real rectangle. */
  private apply(entry: Entry): void {
    const show =
      entry.wantsVisible && !this.obscured && !entry.occluded && entry.bounds !== null;
    if (show && entry.bounds) {
      entry.view.setBounds(entry.bounds);
      if (!entry.shown) {
        entry.view.setVisible(true);
        entry.shown = true;
      }
      return;
    }
    if (entry.shown) {
      entry.view.setVisible(false);
      entry.shown = false;
    }
  }

  private async captureVisibleSnapshots(): Promise<void> {
    if (this.snapshotInFlight) return;
    this.snapshotInFlight = true;
    try {
      for (const [id, entry] of this.pages) {
        if (!entry.shown || entry.occluded) continue;
        await this.captureSnapshot(id, entry);
      }
    } finally {
      this.snapshotInFlight = false;
    }
  }

  private captureSnapshot(id: string, entry: Entry): Promise<string> {
    if (entry.snapshotRequest) return entry.snapshotRequest;
    if (!entry.shown || entry.view.webContents.isDestroyed()) {
      return Promise.resolve(entry.lastSnapshot);
    }
    const request = entry.view.webContents
      .captureSnapshot()
      .then((dataUrl) => {
        if (this.pages.get(id) !== entry || entry.view.webContents.isDestroyed()) return "";
        if (typeof dataUrl === "string" && dataUrl.length > 0) entry.lastSnapshot = dataUrl;
        return entry.lastSnapshot;
      })
      .catch(() => entry.lastSnapshot);
    entry.snapshotRequest = request;
    void request.finally(() => {
      if (entry.snapshotRequest === request) entry.snapshotRequest = null;
    });
    return request;
  }

  private emit(id: string, entry: Entry): void {
    this.options.onState(this.snapshot(id, entry));
  }

  private snapshot(id: string, entry: Entry): BrowserState {
    const page = entry.view.webContents;
    const destroyed = page.isDestroyed();
    const title = destroyed ? "" : page.getTitle().trim();
    return {
      id,
      url: destroyed ? "" : page.getURL(),
      title: title || this.options.fallbackTitle || "Browser",
      canGoBack: !destroyed && page.canGoBack(),
      canGoForward: !destroyed && page.canGoForward(),
      loading: entry.loading,
    };
  }

  private requirePage(id: string): Entry {
    const entry = this.pages.get(id);
    if (!entry) throw new Error("Browser panel is not open");
    return entry;
  }

  private requireId(id: unknown): string {
    const panelId = this.readId(id);
    if (!panelId) throw new Error("Invalid browser panel id");
    return panelId;
  }

  private readId(id: unknown): string | null {
    if (typeof id !== "string") return null;
    this.idPattern.lastIndex = 0;
    if (!this.idPattern.test(id)) return null;
    return id;
  }
}
