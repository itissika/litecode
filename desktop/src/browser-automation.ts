import crypto from "node:crypto";

import {
  BROWSER_AGENT_ACTIVITY_CHANNEL,
  BROWSER_CLOSE_CHANNEL,
  BROWSER_OPEN_CHANNEL,
} from "./bridge-contract";
import {
  BrowserBridgeError,
  type BrowserBridgeHandlers,
} from "./browser-bridge";
import type { BrowserHost, BrowserPageInfo } from "./browser-host";
import { isAllowedLoadUrl } from "./url-policy";

const OPEN_TIMEOUT_MS = 4_000;
const OPEN_READY_TIMEOUT_MS = 12_000;
const CLOSE_TIMEOUT_MS = 2_500;
const PAGE_ACTION_TIMEOUT_MS = 10_000;
const WAIT_STEP_MS = 25;
const MAX_URL_LENGTH = 8_000;
const MAX_TYPE_LENGTH = 4_000;
const MAX_READ_CHARS = 12_000;
const MIN_READ_CHARS = 1_000;
const MAX_READ_CHARS_LIMIT = 30_000;
const REF_PATTERN = /^e[1-9][0-9]{0,3}$/;
const DEFAULT_BUSY_LABEL = "Agent is working on this page…";

type BrowserElement = {
  ref: string;
  tag: string;
  text: string;
  type?: string;
  href?: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}

function requireString(
  input: Record<string, unknown>,
  name: string,
  max: number,
  allowEmpty = false,
): string {
  const value = input[name];
  if (typeof value !== "string") {
    throw new BrowserBridgeError("invalid_input", `${name} must be a string`, 400);
  }
  if (value.length > max) {
    throw new BrowserBridgeError("invalid_input", `${name} is too long`, 400);
  }
  if (!allowEmpty && value.trim().length === 0) {
    throw new BrowserBridgeError("invalid_input", `${name} is required`, 400);
  }
  return value;
}

function optionalPageId(input: Record<string, unknown>): string | undefined {
  const value = input.page_id;
  if (value === undefined || value === null || value === "") return undefined;
  if (typeof value !== "string" || value.length > 100) {
    throw new BrowserBridgeError("invalid_input", "page_id is invalid", 400);
  }
  return value;
}

function requireRef(input: Record<string, unknown>): string {
  const ref = requireString(input, "ref", 16);
  if (!REF_PATTERN.test(ref)) {
    throw new BrowserBridgeError("invalid_input", "ref is invalid; call browser_read first", 400);
  }
  return ref;
}

function requireHost(getHost: () => BrowserHost | null): BrowserHost {
  const host = getHost();
  if (!host) {
    throw new BrowserBridgeError(
      "unavailable",
      "The desktop browser is not available in this session",
      503,
    );
  }
  return host;
}

function resolveVisiblePage(host: BrowserHost, pageId?: string): BrowserPageInfo {
  const id = pageId ?? host.activeId();
  if (!id) {
    throw new BrowserBridgeError("not_found", "No browser tab is active", 404);
  }
  const page = host.pageInfo(id);
  if (!page) {
    throw new BrowserBridgeError("not_found", "Browser tab is not open", 404);
  }
  if (!page.visible) {
    throw new BrowserBridgeError(
      "conflict",
      "Browser tab is hidden; bring it to the front before reading or operating it",
      409,
    );
  }
  return page;
}

async function waitForPage(
  host: BrowserHost,
  id: string,
  timeoutMs: number,
  accept: (page: BrowserPageInfo) => boolean = () => true,
): Promise<BrowserPageInfo | null> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const page = host.pageInfo(id);
    if (page && accept(page)) return page;
    if (Date.now() >= deadline) return null;
    await new Promise<void>((resolve) => setTimeout(resolve, WAIT_STEP_MS));
  }
}

async function withDeadline<T>(
  action: Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  try {
    return await Promise.race([
      action,
      new Promise<T>((_resolve, reject) => {
        timer = setTimeout(
          () => reject(new BrowserBridgeError("timeout", message, 504)),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** Remove a tab the agent opened but could not make usable. */
function discardTab(
  browser: BrowserHost,
  send: BrowserAutomationOptions["send"],
  id: string,
): void {
  browser.clearAgentOwned(id);
  browser.destroy(id);
  try {
    send(BROWSER_CLOSE_CHANNEL, { id });
  } catch {
    // The workbench is gone; the native page is already destroyed.
  }
}

async function waitForClosed(
  host: BrowserHost,
  id: string,
  timeoutMs: number,
): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (!host.pageInfo(id)) return true;
    if (Date.now() >= deadline) return false;
    await new Promise<void>((resolve) => setTimeout(resolve, WAIT_STEP_MS));
  }
}

function readElements(value: unknown): BrowserElement[] {
  if (!Array.isArray(value)) return [];
  const elements: BrowserElement[] = [];
  for (const item of value.slice(0, 100)) {
    if (!isRecord(item)) continue;
    if (typeof item.ref !== "string" || !REF_PATTERN.test(item.ref)) continue;
    if (typeof item.tag !== "string") continue;
    const text = typeof item.text === "string" ? item.text.slice(0, 160) : "";
    const element: BrowserElement = {
      ref: item.ref,
      tag: item.tag.slice(0, 20),
      text,
    };
    if (typeof item.type === "string" && item.type.length > 0) {
      element.type = item.type.slice(0, 30);
    }
    if (typeof item.href === "string" && item.href.length > 0) {
      element.href = item.href.slice(0, 2_000);
    }
    elements.push(element);
  }
  return elements;
}

function jsString(value: string): string {
  return JSON.stringify(value)
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
}

function readScript(maxChars: number): string {
  return `(() => {
    const maxChars = ${Math.max(MIN_READ_CHARS, Math.min(MAX_READ_CHARS_LIMIT, maxChars))};
    if (!document || !document.body) {
      return { url: location.href, title: document?.title || "", text: "", truncated: false, elements: [] };
    }
    for (const node of document.querySelectorAll("[data-litecode-ref]")) {
      node.removeAttribute("data-litecode-ref");
    }
    const elements = [];
    const candidates = document.querySelectorAll(
      'a[href],button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"]'
    );
    let next = 0;
    for (const el of candidates) {
      if (next >= 100) break;
      const style = window.getComputedStyle(el);
      const rect = el.getBoundingClientRect();
      if (style.display === "none" || style.visibility === "hidden") continue;
      if (rect.width <= 0 || rect.height <= 0) continue;
      const ref = "e" + (++next);
      el.setAttribute("data-litecode-ref", ref);
      const tag = el.tagName.toLowerCase();
      const rawText =
        el.getAttribute("aria-label") ||
        el.innerText ||
        el.textContent ||
        el.getAttribute("placeholder") ||
        el.getAttribute("name") ||
        el.getAttribute("value") ||
        "";
      const text = rawText.replace(/\\s+/g, " ").trim().slice(0, 160);
      const item = { ref, tag, text };
      const type = el.getAttribute("type");
      if (type) item.type = type;
      if (tag === "a" && typeof el.href === "string") item.href = el.href;
      elements.push(item);
    }
    const mask = document.querySelector("[data-litecode-agent-mask]");
    if (mask) mask.style.display = "none";
    let text = document.body.innerText || document.body.textContent || "";
    if (mask) mask.style.display = "";
    text = text.replace(/\\n{3,}/g, "\\n\\n").trim();
    const truncated = text.length > maxChars;
    if (truncated) text = text.slice(0, maxChars);
    return {
      url: location.href,
      title: document.title || "",
      text,
      truncated,
      elements,
    };
  })()`;
}

/** Dim the page and swallow pointer input while the agent acts on it. */
function showMaskScript(labelText: string): string {
  return `(() => {
    const existing = document.querySelector("[data-litecode-agent-mask]");
    if (existing) {
      clearTimeout(existing.__litecodeHideTimer);
      return { shown: true };
    }
    const mask = document.createElement("div");
    mask.setAttribute("data-litecode-agent-mask", "");
    mask.setAttribute("role", "status");
    mask.setAttribute("aria-live", "polite");
    mask.style.cssText =
      "position:fixed;inset:0;z-index:2147483647;display:grid;place-items:center;" +
      "background:rgba(0,0,0,0.22);cursor:progress;user-select:none;" +
      "-webkit-user-select:none;font:500 12px/1.4 system-ui,-apple-system,'Segoe UI',sans-serif";
    const pill = document.createElement("div");
    pill.style.cssText =
      "display:inline-flex;align-items:center;gap:8px;padding:8px 14px;" +
      "border-radius:999px;background:rgba(20,20,20,0.92);color:#fff;" +
      "box-shadow:0 4px 20px rgba(0,0,0,0.35)";
    const dot = document.createElement("span");
    dot.style.cssText = "width:8px;height:8px;border-radius:999px;background:#4ade80";
    if (typeof dot.animate === "function") {
      dot.animate([{ opacity: 1 }, { opacity: 0.25 }], {
        duration: 700,
        iterations: Infinity,
        direction: "alternate",
      });
    }
    const label = document.createElement("span");
    label.textContent = ${jsString(labelText)};
    pill.appendChild(dot);
    pill.appendChild(label);
    mask.appendChild(pill);
    const swallow = (event) => event.preventDefault();
    mask.addEventListener("wheel", swallow, { passive: false });
    mask.addEventListener("contextmenu", swallow);
    // Safety net: never leave a page covered if the caller disappears.
    mask.__litecodeTimer = setTimeout(() => mask.remove(), 20000);
    mask.__litecodeShownAt = Date.now();
    document.documentElement.appendChild(mask);
    return { shown: true };
  })()`;
}

function hideMaskScript(): string {
  // Keep the mask visible for a moment so a fast click still reads as
  // "the agent is driving", without delaying the tool result.
  return `(() => {
    const mask = document.querySelector("[data-litecode-agent-mask]");
    if (!mask) return { hidden: true };
    const remaining = Math.max(0, 500 - (Date.now() - (mask.__litecodeShownAt || 0)));
    clearTimeout(mask.__litecodeHideTimer);
    if (remaining === 0) {
      clearTimeout(mask.__litecodeTimer);
      mask.remove();
      return { hidden: true };
    }
    mask.__litecodeHideTimer = setTimeout(() => {
      clearTimeout(mask.__litecodeTimer);
      mask.remove();
    }, remaining);
    return { hidden: false };
  })()`;
}

/** Best-effort: a mask that fails to render must never fail the action. */
function showAgentMask(browser: BrowserHost, id: string, label: string): void {
  void browser.executeJavaScript(id, showMaskScript(label)).catch(() => undefined);
}

function hideAgentMask(browser: BrowserHost, id: string): void {
  void browser.executeJavaScript(id, hideMaskScript()).catch(() => undefined);
}

function clickScript(ref: string): string {
  return `(() => {
    const ref = ${jsString(ref)};
    const el = document.querySelector('[data-litecode-ref="' + ref + '"]');
    if (!el) return { ok: false, message: "The element is no longer on the page; call browser_read again" };
    if (el.getClientRects().length === 0) {
      return { ok: false, message: "The element is not visible; call browser_read again" };
    }
    el.scrollIntoView({ block: "center", inline: "center" });
    if (typeof el.click === "function") {
      el.click();
    } else {
      el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
    }
    const text = (el.innerText || el.textContent || el.getAttribute("aria-label") || "")
      .replace(/\\s+/g, " ").trim().slice(0, 160);
    return { ok: true, tag: el.tagName.toLowerCase(), text };
  })()`;
}

function typeScript(ref: string, text: string): string {
  return `(() => {
    const ref = ${jsString(ref)};
    const nextValue = ${jsString(text)};
    const el = document.querySelector('[data-litecode-ref="' + ref + '"]');
    if (!el) return { ok: false, message: "The element is no longer on the page; call browser_read again" };
    if (el.getClientRects().length === 0) {
      return { ok: false, message: "The element is not visible; call browser_read again" };
    }
    el.scrollIntoView({ block: "center", inline: "center" });
    el.focus();
    if (el.isContentEditable) {
      el.textContent = nextValue;
    } else if (el.tagName === "SELECT") {
      el.value = nextValue;
    } else if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
      const proto =
        el.tagName === "TEXTAREA"
          ? HTMLTextAreaElement.prototype
          : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
      if (setter) setter.call(el, nextValue);
      else el.value = nextValue;
    } else {
      return { ok: false, message: "The element is not a text input; call browser_read again" };
    }
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return { ok: true, tag: el.tagName.toLowerCase() };
  })()`;
}

function activity(
  send: (channel: string, payload: unknown) => void,
  id: string,
  busy: boolean,
): void {
  try {
    send(BROWSER_AGENT_ACTIVITY_CHANNEL, { id, busy });
  } catch {
    // The renderer can disappear mid-action; the native page is still unlocked below.
  }
}

function clickResult(value: unknown): { tag: string; text: string } {
  if (!isRecord(value) || value.ok !== true) {
    const message =
      isRecord(value) && typeof value.message === "string"
        ? value.message
        : "Could not click the element";
    throw new BrowserBridgeError("stale", message, 409);
  }
  return {
    tag: typeof value.tag === "string" ? value.tag : "element",
    text: typeof value.text === "string" ? value.text.slice(0, 160) : "",
  };
}

function typeResult(value: unknown): { tag: string } {
  if (!isRecord(value) || value.ok !== true) {
    const message =
      isRecord(value) && typeof value.message === "string"
        ? value.message
        : "Could not type into the element";
    throw new BrowserBridgeError("stale", message, 409);
  }
  return { tag: typeof value.tag === "string" ? value.tag : "element" };
}

export type BrowserAutomationOptions = {
  getHost(): BrowserHost | null;
  /** Send an IPC event to the workbench renderer. Throws when it is gone. */
  send(channel: string, payload: unknown): void;
  /** Localized label for the "agent is working" page mask. */
  busyLabel?: () => string;
  /** How long an opened tab may take to load before the call fails. */
  openReadyTimeoutMs?: number;
};

/** The shared-page actions behind the local HTTP bridge. */
export function createBrowserAutomation(
  options: BrowserAutomationOptions,
): BrowserBridgeHandlers {
  const host = () => requireHost(options.getHost);

  return {
    async open(input) {
      const url = requireString(input, "url", MAX_URL_LENGTH);
      if (!isAllowedLoadUrl(url)) {
        throw new BrowserBridgeError(
          "invalid_input",
          "Only http and https URLs can be opened",
          400,
        );
      }
      const browser = host();
      const id = `browser-${crypto.randomUUID()}`;
      browser.markAgentOwned(id);
      try {
        options.send(BROWSER_OPEN_CHANNEL, { id, url });
      } catch (error) {
        browser.clearAgentOwned(id);
        throw error;
      }
      const page = await waitForPage(browser, id, OPEN_TIMEOUT_MS);
      if (!page) {
        browser.clearAgentOwned(id);
        throw new BrowserBridgeError(
          "timeout",
          "The browser tab did not open; wait for the workbench to finish restoring and try again",
          504,
        );
      }
      if (page.url.length > 0) return page;
      // browserCreate makes the native page before the panel starts
      // navigating. Wait for real content instead of handing back a blank tab;
      // a tab that never loads is an error, not a success.
      const ready = await waitForPage(
        browser,
        id,
        options.openReadyTimeoutMs ?? OPEN_READY_TIMEOUT_MS,
        (next) => next.url.length > 0,
      );
      if (ready) return ready;
      discardTab(browser, options.send, id);
      throw new BrowserBridgeError(
        "timeout",
        "The browser tab opened but did not load; check the URL and try again",
        504,
      );
    },

    async close(input) {
      const id = requireString(input, "page_id", 100);
      const browser = host();
      if (!browser.isAgentOwned(id)) {
        throw new BrowserBridgeError(
          "permission_denied",
          "Only browser tabs opened by the agent can be closed",
          403,
        );
      }
      if (!browser.pageInfo(id)) {
        browser.clearAgentOwned(id);
        return { id, closed: true };
      }
      try {
        options.send(BROWSER_CLOSE_CHANNEL, { id });
      } catch {
        browser.destroy(id);
        browser.clearAgentOwned(id);
        return { id, closed: true, fallback: true };
      }
      if (!(await waitForClosed(browser, id, CLOSE_TIMEOUT_MS))) {
        // A renderer that lost the panel must not leave an agent-owned page
        // alive forever. The native page is the resource this tool owns.
        browser.destroy(id);
        browser.clearAgentOwned(id);
        return { id, closed: true, fallback: true };
      }
      browser.clearAgentOwned(id);
      return { id, closed: true };
    },

    list() {
      const browser = host();
      return {
        pages: browser.list(),
        active_id: browser.activeId() ?? "",
      };
    },

    async read(input) {
      const browser = host();
      const page = resolveVisiblePage(browser, optionalPageId(input));
      const rawMax = input.max_chars;
      let maxChars = MAX_READ_CHARS;
      if (rawMax !== undefined) {
        if (typeof rawMax !== "number" || !Number.isFinite(rawMax)) {
          throw new BrowserBridgeError("invalid_input", "max_chars must be a number", 400);
        }
        maxChars = Math.max(MIN_READ_CHARS, Math.min(MAX_READ_CHARS_LIMIT, Math.floor(rawMax)));
      }
      const raw = await withDeadline(
        browser.executeJavaScript(page.id, readScript(maxChars)),
        PAGE_ACTION_TIMEOUT_MS,
        "The page did not respond; try again or reload it",
      );
      if (!isRecord(raw)) {
        throw new BrowserBridgeError("unavailable", "The page returned no readable content", 502);
      }
      const text = typeof raw.text === "string" ? raw.text.slice(0, maxChars) : "";
      return {
        page_id: page.id,
        url: typeof raw.url === "string" ? raw.url : page.url,
        title: typeof raw.title === "string" ? raw.title : page.title,
        text,
        truncated: raw.truncated === true || text.length >= maxChars,
        elements: readElements(raw.elements),
      };
    },

    async click(input) {
      const browser = host();
      const page = resolveVisiblePage(browser, optionalPageId(input));
      const ref = requireRef(input);
      browser.setInputBlocked(page.id, true);
      activity(options.send, page.id, true);
      showAgentMask(browser, page.id, options.busyLabel?.() ?? DEFAULT_BUSY_LABEL);
      try {
        const result = clickResult(
          await withDeadline(
            browser.executeJavaScript(page.id, clickScript(ref)),
            PAGE_ACTION_TIMEOUT_MS,
            "The page did not respond; try again or reload it",
          ),
        );
        const current = browser.pageInfo(page.id);
        return {
          page_id: page.id,
          ref,
          tag: result.tag,
          text: result.text,
          url: current?.url ?? page.url,
          title: current?.title ?? page.title,
        };
      } finally {
        hideAgentMask(browser, page.id);
        browser.setInputBlocked(page.id, false);
        activity(options.send, page.id, false);
      }
    },

    async type(input) {
      const browser = host();
      const page = resolveVisiblePage(browser, optionalPageId(input));
      const ref = requireRef(input);
      const text = requireString(input, "text", MAX_TYPE_LENGTH, true);
      browser.setInputBlocked(page.id, true);
      activity(options.send, page.id, true);
      showAgentMask(browser, page.id, options.busyLabel?.() ?? DEFAULT_BUSY_LABEL);
      try {
        const result = typeResult(
          await withDeadline(
            browser.executeJavaScript(page.id, typeScript(ref, text)),
            PAGE_ACTION_TIMEOUT_MS,
            "The page did not respond; try again or reload it",
          ),
        );
        const current = browser.pageInfo(page.id);
        return {
          page_id: page.id,
          ref,
          tag: result.tag,
          url: current?.url ?? page.url,
          title: current?.title ?? page.title,
        };
      } finally {
        hideAgentMask(browser, page.id);
        browser.setInputBlocked(page.id, false);
        activity(options.send, page.id, false);
      }
    },
  };
}
