import fs from "node:fs";
import path from "node:path";

import { dvThemeAttr, readUiTheme, type UiThemeName } from "./ui-theme";

/** Resolve the product's tokens.css (single source of --_dk-* definitions). */
export function loadThemeTokensCss(): string {
  const candidates = [
    path.join(__dirname, "theme-tokens.css"),
    path.resolve(__dirname, "../../web/src/theme/tokens.css"),
    path.resolve(process.cwd(), "../web/src/theme/tokens.css"),
    path.resolve(process.cwd(), "web/src/theme/tokens.css"),
  ];
  for (const candidate of candidates) {
    try {
      return fs.readFileSync(candidate, "utf8");
    } catch {
      // try next
    }
  }
  throw new Error(
    "theme tokens.css not found (expected desktop/dist/theme-tokens.css or web/src/theme/tokens.css)",
  );
}

/**
 * Resolve the LiteCode wordmark font (LexendDeca 600) as a base64 data URI so
 * the file:// hub page can render the same logo as the web workbench without a
 * server. Mirrors loadThemeTokensCss's fallback chain (dev public / built dist).
 * Returns "" when the font cannot be found — the logo then falls back to the UI font.
 */
function loadLogoFontBase64(): string {
  const candidates = [
    path.resolve(__dirname, "..", "..", "web", "public", "fonts", "LexendDeca-600.ttf"),
    path.resolve(__dirname, "..", "..", "web", "dist", "fonts", "LexendDeca-600.ttf"),
    path.resolve(__dirname, "..", "web", "public", "fonts", "LexendDeca-600.ttf"),
    path.resolve(process.cwd(), "web", "public", "fonts", "LexendDeca-600.ttf"),
    path.resolve(process.cwd(), "web", "dist", "fonts", "LexendDeca-600.ttf"),
  ];
  for (const candidate of candidates) {
    try {
      return fs.readFileSync(candidate).toString("base64");
    } catch {
      // try next candidate
    }
  }
  return "";
}

/**
 * Resolve the official app icon (web/public/icon.png) as a base64 data URI so
 * the file:// hub page can render it next to the wordmark (CSP: img-src data:).
 * Mirrors loadLogoFontBase64's fallback chain. Returns "" when not found.
 */
function loadLogoIconBase64(): string {
  const candidates = [
    path.resolve(__dirname, "..", "..", "web", "public", "icon.png"),
    path.resolve(__dirname, "..", "..", "web", "dist", "icon.png"),
    path.resolve(__dirname, "..", "web", "public", "icon.png"),
    path.resolve(process.cwd(), "web", "public", "icon.png"),
    path.resolve(process.cwd(), "web", "dist", "icon.png"),
  ];
  for (const candidate of candidates) {
    try {
      return fs.readFileSync(candidate).toString("base64");
    } catch {
      // try next candidate
    }
  }
  return "";
}

/**
 * Render the LiteCode wordmark the same way the web `Logo` component does
 * (per-letter spans, LogoFont, 600 weight, 0.025em tracking) but with the
 * entrance animation disabled — the hub is a static landing page.
 */
function litecodeLogoHtml(fontSize: number, color: string, marginLeft = "0"): string {
  const letters = "LiteCode"
    .split("")
    .map((ch) => `<span style="font-size:${fontSize}px;color:${color}">${ch}</span>`)
    .join("");
  return `<span class="litecode-logo select-none whitespace-nowrap" style="margin-left:${marginLeft}">${letters}</span>`;
}

function hubLayoutCss(logoFontSrc: string): string {
  const fontFace = logoFontSrc
    ? `
@font-face{
  font-family:"LogoFont";
  src:url("${logoFontSrc}") format("truetype");
  font-weight:600;
  font-display:block;
}`
    : "";
  const logoStyles = `
.litecode-logo{display:inline-flex;align-items:center}
.litecode-logo span{display:inline-block;font-family:LogoFont,var(--_dk-font-ui),system-ui,sans-serif;font-weight:600;letter-spacing:0.025em}
.logo-hero{display:flex;align-items:center;justify-content:center;gap:var(--hub-gap)}
.logo-hero .logo-icon{width:var(--hub-icon);height:var(--hub-icon);flex-shrink:0;display:block}
`;
  return `
${fontFace}
${logoStyles}
*{box-sizing:border-box}
/* Scrollbars: thin, low-contrast — matches web/src/theme/index.css (agent
   panel message list). Applied app-wide so the home list's vertical scrollbar
   uses the same theme styling. */
*{scrollbar-width:thin;scrollbar-color:var(--_dk-line) transparent}
*::-webkit-scrollbar{width:8px;height:8px}
*::-webkit-scrollbar-track{background:transparent;border:0 none}
*::-webkit-scrollbar-thumb{background:var(--_dk-line);border-radius:4px;border:0 none;outline:0 none}
*::-webkit-scrollbar-thumb:hover{background:var(--_dk-text-disabled)}
*::-webkit-scrollbar-corner{background:transparent}
html,body{height:100%;margin:0;overflow:hidden}
body{
  background:var(--_dk-root);
  color:var(--_dk-text-primary);
  font:var(--_dk-text-md)/1.45 var(--_dk-font-ui);
  display:flex;
  flex-direction:column;
  min-height:0;
}
:root{
  --font-sans:system-ui,-apple-system,"Segoe UI",sans-serif;
  --font-mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;
  --radius-sm:0.375rem;
  /* Home layout: the list column is centered and only slightly wider than the
     icon+wordmark logo. --hub-text-w is the measured "LiteCode" width ratio
     (4.623em incl. 0.025em tracking, LexendDeca 600) so the list tracks the
     logo width exactly when --hub-text changes. */
  --hub-icon:56px;
  --hub-gap:14px;
  --hub-text:56px;
  --hub-text-w:4.623;
  --hub-logo-width:calc(var(--hub-icon) + var(--hub-gap) + var(--hub-text) * var(--hub-text-w));
  /* Hand-tuned: percentage-based, clamped. 80% of the container, 340px floor, 666px cap. */
  --hub-list-max:clamp(340px, 80%, 666px);
}
#titlebar{
  height:30px;flex-shrink:0;display:flex;align-items:center;
  border-bottom:1px solid var(--_dk-line-visible);
  background:var(--_dk-header);
  -webkit-app-region:drag;user-select:none;
}
#titlebar .brand{padding:0 12px;font-size:var(--_dk-text-sm);color:var(--_dk-text-muted)}
#titlebar .chrome{margin-left:auto;display:flex;-webkit-app-region:no-drag}
#titlebar .chrome button{
  width:46px;height:30px;border:0;border-radius:0;padding:0;background:transparent;
  color:var(--_dk-text-secondary);cursor:pointer;display:flex;align-items:center;justify-content:center;
}
#titlebar .chrome button:hover{background:var(--_dk-ix-bg-hover);color:var(--_dk-ix-fg-hover)}
#titlebar .chrome #close:hover{color:var(--_dk-close-hover-color);background:var(--_dk-close-hover-bg)}
main{
  flex:1;min-height:0;overflow:hidden;
  max-width:1080px;width:100%;margin:0 auto;
  padding:20px 32px 16px;
  display:flex;flex-direction:column;
}
#home-view,#remote-view{flex:1;min-height:0;display:flex;flex-direction:column}
.home-column{
  flex:1;min-height:0;
  width:100%;max-width:var(--hub-list-max);margin:0 auto;
  display:flex;flex-direction:column;
}
.logo-hero{flex-shrink:0}
.lead{
  flex-shrink:0;color:var(--_dk-text-muted);margin:10px 0 0;
  font-size:var(--_dk-text-md);text-align:center;
}
.home-header{
  flex-shrink:0;display:flex;align-items:center;justify-content:space-between;gap:16px;
  margin-top:28px;
}
.home-title{
  font-size:var(--_dk-text-xl);font-weight:var(--_dk-text-weight-semibold);
  color:var(--_dk-text-primary);margin:0;
}
.home-actions{display:flex;gap:8px}
.home-divider{
  flex-shrink:0;height:1px;background:var(--_dk-line-visible);
  margin:12px 0;
}
h1{font-size:2rem;font-weight:var(--_dk-text-weight-semibold);margin:0 0 8px;color:var(--_dk-text-primary)}
.hidden{display:none !important}
button.btn-primary,button.btn,button.btn-ghost,button.btn-danger,button.btn-icon,button.btn-sm{
  display:inline-flex;align-items:center;justify-content:center;gap:0.375rem;
  border-radius:var(--radius-sm);border:1px solid transparent;background:transparent;
  color:var(--_dk-ix-fg);cursor:pointer;user-select:none;white-space:nowrap;font:inherit;
}
button.btn-primary,button.btn{padding:0.375rem 0.75rem;font-size:0.875rem;line-height:1.25rem}
button.btn-sm{padding:0.25rem 0.5rem;font-size:var(--_dk-text-sm);line-height:1.125rem}
button.btn-icon{padding:0.375rem;aspect-ratio:1}
button.btn-primary{border-color:var(--_dk-line-visible);color:var(--_dk-text-primary)}
button.btn-primary:hover{background:var(--_dk-ix-bg-hover)}
button.btn-primary:active{background:var(--_dk-ix-bg-pressed)}
button.btn{border-color:var(--_dk-line)}
button.btn:hover{background:var(--_dk-ix-bg-hover);color:var(--_dk-ix-fg-hover)}
button.btn:active{background:var(--_dk-ix-bg-pressed)}
button.btn-ghost:hover{background:var(--_dk-ix-bg-hover);color:var(--_dk-ix-fg-hover)}
button.btn-danger{color:var(--_dk-ix-danger-fg)}
button.btn-danger:hover{background:var(--_dk-ix-danger-bg-hover)}
button.btn-danger:active{background:var(--_dk-ix-danger-bg-pressed)}
button:disabled{opacity:0.5;cursor:not-allowed}
#status{flex-shrink:0;min-height:0;margin:0;color:var(--_dk-amber-500);font-size:var(--_dk-text-sm);white-space:pre-wrap;overflow:auto;max-height:220px}
#status:not(:empty){min-height:22px;margin:8px 0 0}
#status.error{color:var(--_dk-red-500)}
.home-list{
  flex:1;min-height:0;overflow-y:auto;overflow-x:hidden;
  /* Horizontal padding gives the hover scale-up (1.015) room to grow so the
     enlarged row is not clipped by overflow-x:hidden. */
  padding:0 8px;
}
.row{
  display:flex;align-items:center;gap:12px;padding:4px 4px;border-radius:var(--radius-sm);
  transition:background-color 120ms ease, transform 120ms ease;
}
.row:hover{background:var(--_dk-ix-bg-hover);transform:scale(1.015)}
.row:active{background:var(--_dk-ix-bg-pressed);transform:scale(0.985)}
.row .path{
  flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:left;
  padding:0;border:0;background:transparent;color:var(--_dk-text-primary);cursor:pointer;font:inherit;
}
.row .path:hover{color:var(--_dk-accent-hover)}
/* Remote meta sits inline on the same line as the label, separated by a dot. */
.row .meta{display:inline;font-size:var(--_dk-text-sm);color:var(--_dk-text-muted);margin-left:8px}
.row .meta::before{content:"·";margin-right:6px;color:var(--_dk-text-muted)}
.tag{
  flex-shrink:0;display:inline-flex;align-items:center;
  padding:1px 8px;border-radius:999px;border:1px solid transparent;
  font-size:var(--_dk-text-2xs);font-weight:var(--_dk-text-weight-medium);
  line-height:1.6;text-transform:uppercase;letter-spacing:0.04em;
}
.tag-local{color:var(--_dk-tag-ok-fg);background:var(--_dk-tag-ok-bg);border-color:var(--_dk-tag-ok-border)}
.tag-remote{color:var(--_dk-tag-warn-fg);background:var(--_dk-tag-warn-bg);border-color:var(--_dk-tag-warn-border)}
.row .pin{flex-shrink:0}
.row .pin.pinned{color:var(--_dk-accent-hover)}
.empty{color:var(--_dk-text-disabled);padding:16px;font-size:var(--_dk-text-sm)}
.panel{
  margin-top:20px;padding:16px;border:1px solid var(--_dk-line-visible);
  border-radius:var(--radius-sm);background:var(--_dk-surface-raised);
}
.panel h2{margin:0 0 12px;font-size:var(--_dk-text-md);font-weight:var(--_dk-text-weight-medium)}
.field{display:block;margin-bottom:12px;font-size:var(--_dk-text-sm);color:var(--_dk-text-muted)}
.field input,.field select{
  display:block;width:100%;margin-top:4px;padding:0.4rem 0.6rem;
  border-radius:var(--radius-sm);border:1px solid var(--_dk-line-visible);
  background:var(--_dk-root);color:var(--_dk-text-primary);font:inherit;
}
.field input:focus,.field select:focus{outline:1px solid var(--_dk-accent-ring)}
.advanced{margin:8px 0 12px}
.advanced summary{cursor:pointer;color:var(--_dk-text-muted);font-size:var(--_dk-text-sm)}
.progress{margin:12px 0}
.progress-track{height:6px;border-radius:999px;background:var(--_dk-line);overflow:hidden}
.progress-bar{height:100%;width:0%;background:var(--_dk-accent-hover);transition:width 160ms ease}
.progress-msg{margin-top:8px;font-size:var(--_dk-text-sm);color:var(--_dk-text-secondary)}
.dir-bar{display:flex;gap:8px;align-items:center;margin-bottom:8px;flex-wrap:wrap}
.dir-list{border:1px solid var(--_dk-line);border-radius:var(--radius-sm);max-height:240px;overflow:auto}
.dir-item{
  display:block;width:100%;text-align:left;padding:8px 10px;border:0;border-bottom:1px solid var(--_dk-line);
  background:transparent;color:var(--_dk-text-primary);cursor:pointer;font:inherit;
}
.dir-item:hover{background:var(--_dk-ix-bg-hover)}
.dir-item.selected{background:var(--_dk-ix-bg-selected);color:var(--_dk-ix-fg-selected)}
.token-box{
  font-family:var(--_dk-font-code);font-size:var(--_dk-text-sm);padding:10px;
  border:1px solid var(--_dk-line-visible);border-radius:var(--radius-sm);
  background:var(--_dk-root);word-break:break-all;color:var(--_dk-text-primary);
}
.panel-actions{display:flex;gap:8px;justify-content:flex-end;margin-top:14px;flex-wrap:wrap}
#remote-view{overflow:auto}
`.trim();
}

function hubScript(): string {
  // Inline script only (file:// hub page CSP allows script-src 'unsafe-inline').
  return `
(function(){
  const bridge = window.litecode;
  const $ = (id) => document.getElementById(id);
  const homeView = $("home-view");
  const remoteView = $("remote-view");
  const statusEl = $("status");
  const list = $("list");
  const maxBtn = $("max");
  const progressBar = $("progress-bar");
  const progressMsg = $("progress-msg");
  const dirList = $("dir-list");
  const dirPath = $("dir-path");
  const tokenBox = $("token-box");

  if (!bridge) {
    statusEl.textContent = "Desktop bridge unavailable. Restart Litecode.";
    statusEl.classList.add("error");
    return;
  }

  let sessionId = null;
  let remotePath = ".";
  let selectedDir = null;
  let stopProgress = null;

  const message = (text, isError) => {
    statusEl.textContent = text || "";
    statusEl.classList.toggle("error", Boolean(isError));
  };

  const show = (el, on) => el.classList.toggle("hidden", !on);
  const setStep = (name) => {
    ["step-auth","step-deploy","step-path","step-token"].forEach((id) => {
      show($(id), id === name);
    });
  };

  const openHome = () => {
    show(homeView, true);
    show(remoteView, false);
    setStep("step-auth");
    sessionId = null;
    selectedDir = null;
    if (stopProgress) { stopProgress(); stopProgress = null; }
  };

  const openRemoteWizard = () => {
    show(homeView, false);
    show(remoteView, true);
    setStep("step-auth");
    $("user-at-host").value = "";
    $("password").value = "";
    $("auth-mode").value = "password";
    $("identity-file").value = "";
    show($("password-field"), true);
    show($("key-field"), false);
    progressBar.style.width = "0%";
    progressMsg.textContent = "";
    message("");
  };

  $("min").onclick = () => void bridge.windowMinimize?.();
  $("close").onclick = () => void bridge.windowClose?.();
  const syncMax = async () => {
    const maximized = await bridge.windowIsMaximized?.();
    maxBtn.setAttribute("aria-label", maximized ? "Restore" : "Maximize");
    maxBtn.title = maximized ? "Restore" : "Maximize";
  };
  maxBtn.onclick = async () => { await bridge.windowMaximizeToggle?.(); syncMax(); };
  void syncMax();

  const openLocal = async (target) => {
    try {
      message("Opening workspace…");
      await bridge.openWorkspace(target);
      message("");
    } catch (error) {
      message(error instanceof Error ? error.message : String(error), true);
    }
  };

  $("open-local").onclick = async () => {
    try {
      const folder = await bridge.pickFolder();
      if (folder) await openLocal(folder);
    } catch (error) {
      message(error instanceof Error ? error.message : String(error), true);
    }
  };
  $("open-remote").onclick = () => openRemoteWizard();
  $("remote-cancel").onclick = async () => {
    try {
      if (sessionId) await bridge.cancelRemoteSession?.(sessionId);
    } catch (_) {}
    openHome();
    render();
  };
  $("remote-cancel-path").onclick = $("remote-cancel").onclick;
  $("remote-back-home").onclick = $("remote-cancel").onclick;

  $("auth-mode").onchange = () => {
    const mode = $("auth-mode").value;
    show($("password-field"), mode === "password");
    show($("key-field"), mode === "private_key");
  };

  const bindProgress = () => {
    if (stopProgress) stopProgress();
    stopProgress = bridge.onRemoteProgress?.((ev) => {
      progressBar.style.width = Math.round((ev.ratio || 0) * 100) + "%";
      progressMsg.textContent = ev.message || "";
    }) || null;
  };

  $("remote-connect").onclick = async () => {
    const userAtHost = $("user-at-host").value.trim();
    const authMode = $("auth-mode").value;
    const password = $("password").value;
    const identityFile = $("identity-file").value.trim();
    if (!userAtHost) { message("Enter user@host.", true); return; }
    setStep("step-deploy");
    bindProgress();
    progressBar.style.width = "4%";
    progressMsg.textContent = "Starting…";
    message("");
    try {
      const result = await bridge.startRemoteSession({
        userAtHost,
        authMode,
        password: authMode === "password" ? password : undefined,
        identityFile: authMode === "private_key" ? identityFile : undefined,
      });
      sessionId = result.sessionId;
      remotePath = ".";
      selectedDir = null;
      await refreshDirs();
      setStep("step-path");
      message("Connected. Choose a workspace folder.");
    } catch (error) {
      setStep("step-auth");
      message(error instanceof Error ? error.message : String(error), true);
    }
  };

  const refreshDirs = async () => {
    const result = await bridge.listPendingRemoteDirs(sessionId, remotePath);
    remotePath = result.path;
    dirPath.textContent = result.path === "." ? result.home : (result.home.replace(/\\/$/, "") + "/" + result.path);
    dirList.replaceChildren();
    if (remotePath !== ".") {
      const up = document.createElement("button");
      up.type = "button";
      up.className = "dir-item";
      up.textContent = "..";
      up.onclick = async () => {
        const parts = remotePath.split("/").filter(Boolean);
        parts.pop();
        remotePath = parts.join("/") || ".";
        selectedDir = null;
        await refreshDirs();
      };
      dirList.append(up);
    }
    for (const entry of result.entries) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "dir-item";
      btn.textContent = entry.name;
      btn.onclick = () => {
        [...dirList.querySelectorAll(".dir-item")].forEach((el) => el.classList.remove("selected"));
        btn.classList.add("selected");
        selectedDir = entry.name;
      };
      btn.ondblclick = async () => {
        remotePath = remotePath === "." ? entry.name : remotePath + "/" + entry.name;
        selectedDir = null;
        await refreshDirs();
      };
      dirList.append(btn);
    }
  };

  $("dir-open").onclick = async () => {
    if (!selectedDir) { message("Select a folder, or double-click to enter it.", true); return; }
    remotePath = remotePath === "." ? selectedDir : remotePath + "/" + selectedDir;
    selectedDir = null;
    await refreshDirs();
  };

  $("dir-use").onclick = async () => {
    const workspace = selectedDir
      ? (remotePath === "." ? selectedDir : remotePath + "/" + selectedDir)
      : remotePath;
    if (!workspace || workspace === ".") {
      message("Choose a workspace folder below the remote home.", true);
      return;
    }
    bindProgress();
    try {
      message("Starting remote server…");
      const done = await bridge.completeRemoteSession(sessionId, workspace);
      tokenBox.textContent = done.token;
      setStep("step-token");
      message("Token generated automatically. Copy if needed, then enter the workspace.");
    } catch (error) {
      message(error instanceof Error ? error.message : String(error), true);
    }
  };

  $("token-copy").onclick = async () => {
    try {
      await navigator.clipboard.writeText(tokenBox.textContent || "");
      message("Token copied.");
    } catch (_) {
      message("Could not copy token.", true);
    }
  };

  $("token-enter").onclick = async () => {
    try {
      message("Opening workspace…");
      await bridge.enterRemoteWorkbench(sessionId);
    } catch (error) {
      message(error instanceof Error ? error.message : String(error), true);
    }
  };

  function pinIconSvg(filled) {
    const fill = filled ? ' fill="currentColor"' : ' fill="none"';
    return '<svg width="14" height="14" viewBox="0 0 24 24"' + fill +
      ' stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">' +
      '<path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z"/></svg>';
  }

  /** Build one merged workspace row: kind tag + left-aligned text + pin icon + remove. */
  function buildRow({ kind, title, meta, pinned, onOpen, onPin, onRemove }) {
    const el = document.createElement("div");
    el.className = "row";

    const tag = document.createElement("span");
    tag.className = "tag tag-" + kind;
    tag.textContent = kind;

    const openButton = document.createElement("button");
    openButton.type = "button";
    openButton.className = "path";
    openButton.title = title;
    if (meta) {
      openButton.innerHTML = "<span>" + escapeHtml(title) + '</span><span class="meta">' + escapeHtml(meta) + "</span>";
    } else {
      openButton.textContent = title;
    }
    openButton.onclick = onOpen;

    const pinBtn = document.createElement("button");
    pinBtn.type = "button";
    pinBtn.className = "btn-ghost btn-icon btn-sm pin" + (pinned ? " pinned" : "");
    pinBtn.title = pinned ? "Unpin" : "Pin";
    pinBtn.setAttribute("aria-label", pinned ? "Unpin" : "Pin");
    pinBtn.innerHTML = pinIconSvg(pinned);
    pinBtn.onclick = onPin;

    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "btn-danger btn-sm";
    remove.textContent = "Remove";
    remove.onclick = onRemove;

    el.append(tag, openButton, pinBtn, remove);
    return el;
  }

  function escapeHtml(value) {
    return String(value)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  async function render() {
    try {
      const [localRows, remoteRows] = await Promise.all([
        bridge.listRecents(),
        bridge.listRemoteHistory(),
      ]);
      list.replaceChildren();
      if (!localRows.length && !remoteRows.length) {
        const empty = document.createElement("div");
        empty.className = "empty";
        empty.textContent = "No workspaces yet. Open a local folder, or connect a remote machine over SSH.";
        list.append(empty);
        return;
      }
      // Merge local + remote into one list, pinned items always first (across
      // both kinds) so pinning actually brings a workspace to the front.
      const items = [];
      for (const row of localRows) {
        items.push({
          pinned: row.pinned,
          lastUsedAt: row.lastOpenedAt,
          build: () => buildRow({
            kind: "local",
            title: row.path,
            meta: "",
            pinned: row.pinned,
            onOpen: () => openLocal(row.path),
            onPin: async () => { await bridge.setRecentPinned(row.path, !row.pinned); render(); },
            onRemove: async () => { await bridge.removeRecent(row.path); render(); },
          }),
        });
      }
      for (const row of remoteRows) {
        items.push({
          pinned: Boolean(row.pinned),
          lastUsedAt: row.lastConnectedAt ?? 0,
          build: () => buildRow({
            kind: "remote",
            title: row.label,
            meta: row.lastWorkspace || "",
            pinned: Boolean(row.pinned),
            onOpen: async () => {
              try {
                message("Reconnecting to " + row.label + "…");
                bindProgress();
                await bridge.reconnectRemote(row.id);
              } catch (error) {
                message(error instanceof Error ? error.message : String(error), true);
              }
            },
            onPin: async () => { await bridge.setRemoteHistoryPinned(row.id, !row.pinned); render(); },
            onRemove: async () => { await bridge.removeSshTarget(row.id); render(); },
          }),
        });
      }
      items.sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.lastUsedAt - a.lastUsedAt);
      for (const item of items) list.append(item.build());
    } catch (error) {
      message(error instanceof Error ? error.message : String(error), true);
    }
  }

  openHome();
  render();
})();
`.trim();
}

/**
 * Startup Home: Open local / Open remote + histories.
 * Styles consume web/src/theme/tokens.css — no hardcoded palette.
 *
 * Returns full HTML (not a data: URL). Main writes this to disk and
 * `loadFile`s it so the Electron preload bridge is available — `data:`
 * pages do not reliably receive `window.litecode`.
 */
export function buildHubHtml(theme: UiThemeName = readUiTheme()): string {
  const tokens = loadThemeTokensCss();
  const dvTheme = dvThemeAttr(theme);
  const logoFontB64 = loadLogoFontBase64();
  const logoFontSrc = logoFontB64 ? `data:font/ttf;base64,${logoFontB64}` : "";
  const logoIconB64 = loadLogoIconBase64();
  const logoIconHtml = logoIconB64
    ? `<img class="logo-icon" src="data:image/png;base64,${logoIconB64}" alt="" draggable="false" />`
    : "";
  return `<!doctype html>
<html lang="en" data-dv-theme="${dvTheme}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src data:; img-src data:">
<title>Litecode</title>
<style>
${tokens}
${hubLayoutCss(logoFontSrc)}
</style></head><body>
<header id="titlebar"><span class="brand">${litecodeLogoHtml(13, "var(--_dk-text-secondary)")}</span><div class="chrome">
<button type="button" class="btn-ghost" id="min" aria-label="Minimize" title="Minimize"><svg width="10" height="10" viewBox="0 0 10 10"><rect y="4" width="10" height="1.5" fill="currentColor"/></svg></button>
<button type="button" class="btn-ghost" id="max" aria-label="Maximize" title="Maximize"><svg width="10" height="10" viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" rx="0.5" fill="none" stroke="currentColor" stroke-width="1.5"/></svg></button>
<button type="button" class="btn-ghost" id="close" aria-label="Close" title="Close"><svg width="10" height="10" viewBox="0 0 10 10"><path d="M1 1l8 8M9 1L1 9" stroke="currentColor" stroke-width="1.5"/></svg></button>
</div></header>
<main>
<div id="status" role="status"></div>

<section id="home-view">
  <div class="home-column">
    <div class="logo-hero">${logoIconHtml}${litecodeLogoHtml(56, "var(--_dk-text-primary)")}</div>
    <p class="lead">Open a local folder, or connect a remote machine over SSH.</p>
    <div class="home-header">
      <h1 class="home-title">workspace</h1>
      <div class="home-actions">
        <button type="button" class="btn-primary" id="open-local">Local</button>
        <button type="button" class="btn" id="open-remote">Remote</button>
      </div>
    </div>
    <div class="home-divider"></div>
    <div id="list" class="home-list"></div>
  </div>
</section>

<section id="remote-view" class="hidden">
  <h1>Open remote</h1>
  <p class="lead">Connect with user@host, deploy Litecode automatically, then choose a workspace.</p>
  <div class="panel" id="step-auth">
    <h2>1. Connect</h2>
    <label class="field">user@host
      <input id="user-at-host" type="text" placeholder="user@192.168.1.10" autocomplete="off" spellcheck="false" />
    </label>
    <label class="field" id="password-field">Password
      <input id="password" type="password" autocomplete="off" />
    </label>
    <details class="advanced">
      <summary>Advanced authentication</summary>
      <label class="field">Mode
        <select id="auth-mode">
          <option value="password" selected>Password</option>
          <option value="private_key">Private key file</option>
          <option value="agent">SSH agent</option>
        </select>
      </label>
      <label class="field hidden" id="key-field">Private key path
        <input id="identity-file" type="text" placeholder="C:\\Users\\you\\.ssh\\id_ed25519" autocomplete="off" spellcheck="false" />
      </label>
    </details>
    <div class="panel-actions">
      <button type="button" class="btn-ghost" id="remote-back-home">Cancel</button>
      <button type="button" class="btn-primary" id="remote-connect">Connect</button>
    </div>
  </div>

  <div class="panel hidden" id="step-deploy">
    <h2>2. Deploy</h2>
    <div class="progress">
      <div class="progress-track"><div class="progress-bar" id="progress-bar"></div></div>
      <div class="progress-msg" id="progress-msg">Preparing…</div>
    </div>
    <div class="panel-actions">
      <button type="button" class="btn-ghost" id="remote-cancel">Cancel</button>
    </div>
  </div>

  <div class="panel hidden" id="step-path">
    <h2>3. Choose workspace</h2>
    <div class="dir-bar">
      <span id="dir-path" class="meta"></span>
      <button type="button" class="btn-sm btn" id="dir-open">Open selected</button>
    </div>
    <div class="dir-list" id="dir-list"></div>
    <div class="panel-actions">
      <button type="button" class="btn-ghost" id="remote-cancel-path">Cancel</button>
      <button type="button" class="btn-primary" id="dir-use">Use this folder</button>
    </div>
  </div>

  <div class="panel hidden" id="step-token">
    <h2>4. Session token</h2>
    <p class="lead">Generated automatically for this session. You do not need to type it.</p>
    <div class="token-box" id="token-box"></div>
    <div class="panel-actions">
      <button type="button" class="btn" id="token-copy">Copy token</button>
      <button type="button" class="btn-primary" id="token-enter">Enter workspace</button>
    </div>
  </div>
</section>
</main>
<script>
${hubScript()}
</script></body></html>`;
}

/** Write hub HTML to `filePath` and return that path for `BrowserWindow.loadFile`. */
export function writeHubPage(filePath: string, theme: UiThemeName = readUiTheme()): string {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, buildHubHtml(theme), "utf8");
  return filePath;
}
