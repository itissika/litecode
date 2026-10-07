/**
 * Owns embedded browser pages for the workbench. Electron view creation stays
 * in `browser-view.ts`; this module only tracks identity, bounds, and navigation
 * so it can be tested without a window.
 */

export const BROWSER_PARTITION = "persist:litecode-browser";
export const BROWSER_STATE_CHANNEL = "litecode:browser-state";

export const BROWSER_ID_PATTERN =
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

export interface BrowserPage {
  loadURL(url: string): Promise<void>;
  getURL(): string;
  getTitle(): string;
  goBack(): void;
  goForward(): void;
  canGoBack(): boolean;
  canGoForward(): boolean;
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
  isAllowedUrl: (url: string) => boolean;
  createView: () => BrowserViewHandle;
  onState: (state: BrowserState) => void;
  /**
   * Move a view onto the main window (`null`) or a registered popout.
   * Absent means the view stays on the window it was created on.
   * False means the move did not happen.
   */
  moveView?: (view: BrowserViewHandle, popoutId: string | null) => boolean;
}

type Entry = {
  view: BrowserViewHandle;
  wantsVisible: boolean;
  shown: boolean;
  bounds: BrowserBounds | null;
  loading: boolean;
  /** `"main"` or the popout dock id the view is attached to. */
  place: string;
};

export function normalizeBrowserBackground(color: unknown): string {
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

export function sanitizeBrowserBounds(value: unknown): BrowserBounds | null {
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
  private obscured = false;

  constructor(private readonly options: BrowserHostOptions) {}

  create(id: string, backgroundColor: unknown): BrowserState {
    const panelId = this.requireId(id);
    const existing = this.pages.get(panelId);
    if (existing) return this.snapshot(panelId, existing);

    const view = this.options.createView();
    view.setBackgroundColor(normalizeBrowserBackground(backgroundColor));
    view.setVisible(false);
    const entry: Entry = {
      view,
      wantsVisible: false,
      shown: false,
      bounds: null,
      loading: false,
      place: "main",
    };
    this.pages.set(panelId, entry);
    this.bind(panelId, entry);
    view.attach();
    return this.snapshot(panelId, entry);
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
    entry.shown = false;
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
    if (!this.options.isAllowedUrl(url)) {
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
    const show = entry.wantsVisible && !this.obscured && entry.bounds !== null;
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
      title: title || "Browser",
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
    if (typeof id !== "string" || !BROWSER_ID_PATTERN.test(id)) return null;
    return id;
  }
}
