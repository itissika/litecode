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
.logo-hero{display:flex;justify-content:center;margin:4px 0 8px}
`;
  return `
${fontFace}
${logoStyles}
*{box-sizing:border-box}
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
  padding:28px 32px 24px;
  display:flex;flex-direction:column;
}
#home-view,#remote-view{flex:1;min-height:0;display:flex;flex-direction:column}
.home-hero{flex-shrink:0}
h1{font-size:2rem;font-weight:var(--_dk-text-weight-semibold);margin:0 0 8px;color:var(--_dk-text-primary)}
.lead{color:var(--_dk-text-muted);margin:0 0 16px;font-size:var(--_dk-text-md)}
.actions{display:flex;flex-wrap:wrap;gap:10px;margin-bottom:4px;justify-content:center}
.home-columns{
  flex:1;min-height:0;
  display:grid;grid-template-columns:1fr 1fr;grid-template-rows:minmax(0,1fr);
  gap:20px;align-items:stretch;margin-top:8px;
}
.hidden{display:none !important}
button.btn-primary,button.btn,button.btn-ghost,button.btn-sm{
  display:inline-flex;align-items:center;justify-content:center;gap:0.375rem;
  border-radius:var(--radius-sm);border:1px solid transparent;background:transparent;
  color:var(--_dk-ix-fg);cursor:pointer;user-select:none;white-space:nowrap;font:inherit;
}
button.btn-primary,button.btn{padding:0.375rem 0.75rem;font-size:0.875rem;line-height:1.25rem}
button.btn-sm{padding:0.25rem 0.5rem;font-size:var(--_dk-text-sm);line-height:1.125rem}
button.btn-primary{border-color:var(--_dk-line-visible);color:var(--_dk-text-primary)}
button.btn-primary:hover{background:var(--_dk-ix-bg-hover)}
button.btn-primary:active{background:var(--_dk-ix-bg-pressed)}
button.btn{border-color:var(--_dk-line)}
button.btn:hover{background:var(--_dk-ix-bg-hover);color:var(--_dk-ix-fg-hover)}
button.btn:active{background:var(--_dk-ix-bg-pressed)}
button.btn-ghost:hover{background:var(--_dk-ix-bg-hover);color:var(--_dk-ix-fg-hover)}
button:disabled{opacity:0.5;cursor:not-allowed}
#status{flex-shrink:0;min-height:0;margin:0;color:var(--_dk-amber-500);font-size:var(--_dk-text-sm)}
#status:not(:empty){min-height:22px;margin:8px 0 0}
#status.error{color:var(--_dk-red-500)}
.recent{
  margin:0;min-height:0;height:100%;
  display:flex;flex-direction:column;overflow:hidden;
}
.recent h2{flex-shrink:0;margin:0 0 8px;font-size:var(--_dk-text-md);font-weight:var(--_dk-text-weight-medium);color:var(--_dk-text-secondary)}
.recent-list{
  flex:1;min-height:0;overflow:auto;
  border:1px solid var(--_dk-line);border-radius:var(--radius-sm);
}
.row{display:flex;align-items:center;gap:10px;padding:12px;border-top:1px solid var(--_dk-line)}
.recent-list .row:first-child{border-top:0}
.row .path{flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:left}
.row .meta{display:block;font-size:var(--_dk-text-sm);color:var(--_dk-text-muted);margin-top:2px}
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
@media (max-width:720px){
  .home-columns{grid-template-columns:1fr;grid-template-rows:minmax(0,1fr) minmax(0,1fr)}
}
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
  const remoteList = $("remote-list");
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

  async function renderLocal() {
    const rows = await bridge.listRecents();
    list.replaceChildren();
    if (!rows.length) {
      const empty = document.createElement("div");
      empty.className = "empty";
      empty.textContent = "No local workspaces yet.";
      list.append(empty);
      return;
    }
    for (const row of rows) {
      const el = document.createElement("div");
      el.className = "row";
      const openButton = document.createElement("button");
      openButton.className = "btn path";
      openButton.textContent = row.path;
      openButton.title = row.path;
      openButton.onclick = () => openLocal(row.path);
      const pin = document.createElement("button");
      pin.className = "btn-ghost btn-sm";
      pin.textContent = row.pinned ? "Unpin" : "Pin";
      pin.onclick = async () => { await bridge.setRecentPinned(row.path, !row.pinned); render(); };
      const remove = document.createElement("button");
      remove.className = "btn-ghost btn-sm";
      remove.textContent = "Remove";
      remove.onclick = async () => { await bridge.removeRecent(row.path); render(); };
      el.append(openButton, pin, remove);
      list.append(el);
    }
  }

  async function renderRemote() {
    const rows = await bridge.listRemoteHistory();
    remoteList.replaceChildren();
    if (!rows.length) {
      const empty = document.createElement("div");
      empty.className = "empty";
      empty.textContent = "No remotes yet. Use Open remote to connect once.";
      remoteList.append(empty);
      return;
    }
    for (const row of rows) {
      const el = document.createElement("div");
      el.className = "row";
      const openButton = document.createElement("button");
      openButton.className = "btn path";
      openButton.innerHTML = "<span>" + escapeHtml(row.label) + '</span><span class="meta">' + escapeHtml(row.lastWorkspace || "") + "</span>";
      openButton.title = row.label;
      openButton.onclick = async () => {
        try {
          message("Reconnecting to " + row.label + "…");
          bindProgress();
          await bridge.reconnectRemote(row.id);
        } catch (error) {
          message(error instanceof Error ? error.message : String(error), true);
        }
      };
      const pin = document.createElement("button");
      pin.className = "btn-ghost btn-sm";
      pin.textContent = row.pinned ? "Unpin" : "Pin";
      pin.onclick = async () => { await bridge.setRemoteHistoryPinned(row.id, !row.pinned); render(); };
      const remove = document.createElement("button");
      remove.className = "btn-ghost btn-sm";
      remove.textContent = "Remove";
      remove.onclick = async () => { await bridge.removeSshTarget(row.id); render(); };
      el.append(openButton, pin, remove);
      remoteList.append(el);
    }
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
      await renderLocal();
      await renderRemote();
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
  <div class="home-hero">
    <div class="logo-hero">${litecodeLogoHtml(80, "var(--_dk-text-primary)")}</div>
    <p class="lead" style="text-align:center">Open a local folder, or connect a remote machine over SSH.</p>
    <div class="actions">
      <button type="button" class="btn-primary" id="open-local">Open local</button>
      <button type="button" class="btn" id="open-remote">Open remote</button>
    </div>
  </div>
  <div class="home-columns">
    <section class="recent"><h2>Local history</h2><div id="list" class="recent-list"></div></section>
    <section class="recent"><h2>Remote history</h2><div id="remote-list" class="recent-list"></div></section>
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
