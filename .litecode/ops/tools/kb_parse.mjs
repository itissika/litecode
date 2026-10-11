// ops.kb_check helper: Node port of litecode's knowledge parsing and resolution.
// Sources mirrored (do not diverge; conformance: tests/fixtures/knowledge_refs/cases.json):
//   src/knowledge/document.rs  parse_knowledge_markdown / file_stem
//   src/knowledge/mentions.rs  extract_cites, scan_prose, masks, parse_line_span, ref_key
//   src/knowledge/validate.rs  resolve (declaration issues + one verdict per ref_key)
//   src/knowledge/fsref.rs     exact-name lookup, symlink escape, read policy, line_count
//   src/knowledge/index.rs     corpus walk, folders/unknown, HEAD drift (git CLI instead of git2)
//   src/knowledge/corpus.rs    unknown files, node ids
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { listScopes, matchScopes, supportsScopes, languageOf, rtrim } from "./kb_scopes.mjs";

export const MAX_TEXT_BYTES = 10 * 1024 * 1024;
export const INVALID_LINES_FORMAT = "invalid line span";
export const INVALID_LINES_ON_SYMBOL = "lines not allowed on symbol reference";

// ---------------------------------------------------------------- mentions.rs

const KEY_RE = /^[\p{L}\p{N}_][\p{L}\p{N}_-]*(?: [\p{L}\p{N}_-]+)*$/u;
const SHORTCODE = /\[@ key="([^"]*)"\]/g;
const FILE_ATTR = /\[@ file="([^"]*)"(?: symbol="([^"]*)")?(?: lines="([^"]*)")?\]/g;

export const normalizeKey = (k) => rtrim(k);
export const isKnowledgeKey = (k) => KEY_RE.test(k);
export const normalizeRefPath = (p) => rtrim(p).replace(/\\/g, "/");

export function isWorkspaceFileRef(p) {
  const slash = p.replace(/\\/g, "/");
  const chars = Array.from(slash);
  if (slash.length === 0 || chars.length > 512) return false;
  if (slash.startsWith("/") || chars[1] === ":") return false;
  return slash.split("/").every((part) => part !== "" && part !== "." && part !== "..");
}

function parseLineNumber(text) {
  if (text === "" || text.startsWith("0") || !/^[0-9]+$/.test(text)) return null;
  const n = Number(text);
  return text.length > 10 || n > 4294967295 ? null : n;
}
export function parseLineSpan(raw) {
  let start, end;
  const at = raw.indexOf("-");
  if (at >= 0) { start = parseLineNumber(raw.slice(0, at)); end = parseLineNumber(raw.slice(at + 1)); if (start === null || end === null) return null; }
  else { start = parseLineNumber(raw); if (start === null) return null; end = start; }
  if (end < start) return null;
  return { start, end };
}
export const formatLineSpan = (s) => (s.start === s.end ? `${s.start}` : `${s.start}-${s.end}`);

const utf8Len = (ch) => { const c = ch.codePointAt(0); return c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4; };
function findCloser(chars, from, ticks) {
  for (let i = from; i + ticks <= chars.length; i++) {
    let ok = true;
    for (let k = 0; k < ticks; k++) if (chars[i + k] !== "`") { ok = false; break; }
    if (ok) return i;
  }
  return -1;
}
// mask_inline_code_same_bytes: masked chars become as many spaces as their UTF-8 bytes.
// Returns masked string plus a map from masked index to source index (UTF-16 units).
function maskInlineCode(line) {
  const chars = Array.from(line);
  let out = "";
  const map = [];
  let src = 0; // utf16 offset of chars[i]
  const offs = [];
  for (const c of chars) { offs.push(src); src += c.length; }
  offs.push(src);
  let i = 0;
  const keep = (k) => { map.push(offs[k]); out += chars[k]; for (let x = 1; x < chars[k].length; x++) map.push(offs[k] + x); };
  while (i < chars.length) {
    if (chars[i] !== "`") { keep(i); i++; continue; }
    let ticks = 0;
    while (i + ticks < chars.length && chars[i + ticks] === "`") ticks++;
    const close = findCloser(chars, i + ticks, ticks);
    if (close < 0) { for (let k = i; k < chars.length; k++) keep(k); break; }
    for (let k = i; k < close + ticks; k++) for (let b = 0; b < utf8Len(chars[k]); b++) { out += " "; map.push(offs[k]); }
    i = close + ticks;
  }
  map.push(offs[chars.length]);
  return { masked: out, map };
}
function fenceOpen(line) {
  const rest = line.replace(/^ +/, "");
  if (line.length - rest.length > 3 || rest.length === 0) return null;
  const ch = rest[0];
  if (ch !== "`" && ch !== "~") return null;
  let len = 0;
  while (len < rest.length && rest[len] === ch) len++;
  return len >= 3 ? { ch, len } : null;
}
function fenceClosed(line, ch, flen) {
  const rest = line.replace(/^ +/, "");
  if (line.length - rest.length > 3) return false;
  let len = 0;
  while (len < rest.length && rest[len] === ch) len++;
  if (len < flen) return false;
  return rtrim(rest.slice(len)) === "";
}
/** Prose lines (outside fences): {line, masked, map, lineNo}. */
export function scanProse(markdown) {
  const out = [];
  let fch = "", flen = 0;
  const raws = markdown.split("\n");
  raws.forEach((raw, idx) => {
    const line = raw.replace(/\r+$/, "");
    if (flen > 0) { if (fenceClosed(line, fch, flen)) flen = 0; return; }
    const f = fenceOpen(line);
    if (f) { fch = f.ch; flen = f.len; return; }
    const { masked, map } = maskInlineCode(line);
    out.push({ line, masked, map, lineNo: idx + 1 });
  });
  return out;
}

function fileAttrCite(m, raw) {
  const p = rtrim(m[1] ?? "");
  if (!p) return null;
  const symRaw = m[2];
  const symbol = symRaw !== undefined && rtrim(symRaw) !== "" ? rtrim(symRaw) : null;
  const linesRaw = m[3];
  const lines = symbol ? null : (linesRaw !== undefined ? parseLineSpan(linesRaw) : null);
  const bad_lines = linesRaw !== undefined && lines === null ? linesRaw : null;
  const kind = symbol ? "symbol" : linesRaw !== undefined ? "range" : "file";
  return { kind, key: null, path: p, symbol, lines, bad_lines, raw };
}

/** extract_cites, with prose line numbers and spans (for ops checks). */
export function extractCites(markdown, withPos = false) {
  const refs = [];
  for (const { line, masked, map, lineNo } of scanProse(markdown)) {
    const hits = [];
    for (const m of masked.matchAll(SHORTCODE)) {
      const key = normalizeKey(m[1] ?? "");
      if (!key || !isKnowledgeKey(key)) continue;
      const s = map[m.index], e = map[m.index + m[0].length];
      hits.push([m.index, { kind: "node", key, path: null, symbol: null, lines: null, bad_lines: null, raw: line.slice(s, e), ...(withPos ? { lineNo, s, e } : {}) }]);
    }
    for (const m of masked.matchAll(FILE_ATTR)) {
      const s = map[m.index], e = map[m.index + m[0].length];
      const c = fileAttrCite(m, line.slice(s, e));
      if (!c) continue;
      if (withPos) Object.assign(c, { lineNo, s, e });
      hits.push([m.index, c]);
    }
    hits.sort((a, b) => a[0] - b[0]);
    for (const [, c] of hits) refs.push(c);
  }
  return refs;
}
export function normPath(c) { return c.path === null ? null : normalizeRefPath(c.path); }
export function refKey(c) {
  const p = normPath(c) ?? "";
  switch (c.kind) {
    case "node": return `node:${c.key ?? ""}`;
    case "file": return `file:${p}`;
    case "symbol": return `sym:${p}\0${c.symbol ?? ""}`;
    default: return `range:${p}\0${c.lines ? formatLineSpan(c.lines) : (c.bad_lines ?? "")}`;
  }
}
/** extract_markers: distinct node keys in prose. */
export function extractMarkers(body) {
  const seen = new Set(); const out = [];
  for (const { masked } of scanProse(body)) for (const m of masked.matchAll(SHORTCODE)) {
    const k = normalizeKey(m[1] ?? "");
    if (!k || !isKnowledgeKey(k) || seen.has(k)) continue;
    seen.add(k); out.push(k);
  }
  return out;
}

// ---------------------------------------------------------------- document.rs

const OPEN_FENCE = /^(?:[ \t]*\r?\n)*```node[ \t]*\r?\n/;
const CLOSE_FENCE = /\r?\n```[ \t]*(?:\r?\n|$)/;
const FIELD_LINE = /^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*?)\s*$/u;
const STATUSES = new Set(["enabled", "disabled", "pending"]);

export function normalizeRel(p) { return p.replace(/\\/g, "/").replace(/^\/+/, "").replace(/\/+$/, ""); }
export function fileStem(p) {
  const base = p.split("/").pop();
  return base.length >= 3 && base.slice(-3).toLowerCase() === ".md" ? base.slice(0, -3) : base;
}
function folderIdOf(p) { const s = p.lastIndexOf("/"); return s <= 0 ? null : p.slice(0, s); }

export function parseKnowledgeMarkdown(p, markdown) {
  const filePath = normalizeRel(p);
  const source = markdown.replace(/^\ufeff+/, "");
  let key = "", status = "enabled", invalid_status = null, summary = "", body = source;
  const coords = { x: null, y: null, w: null, h: null };
  const extras = [];
  let declLines = 0;
  const open = OPEN_FENCE.exec(source);
  if (open) {
    const afterOpen = source.slice(open[0].length);
    const close = CLOSE_FENCE.exec(afterOpen);
    if (close) {
      const block = afterOpen.slice(0, close.index);
      let rest = afterOpen.slice(close.index + close[0].length);
      if (rest.startsWith("\r\n")) rest = rest.slice(2); else if (rest.startsWith("\n") || rest.startsWith("\r")) rest = rest.slice(1);
      body = rest;
      declLines = source.length - body.length; // chars consumed by the declaration
      let sawNode = false;
      for (const line of block.split("\n")) {
        const trimmed = rtrim(rtrim(line).replace(/\r+$/, ""));
        if (!trimmed) continue;
        const f = FIELD_LINE.exec(trimmed);
        if (!f) { extras.push(trimmed); continue; }
        const name = f[1], value = rtrim(f[2] ?? "");
        if (name === "node" && !sawNode) { sawNode = true; key = value; }
        else if (name === "status") { if (STATUSES.has(value)) { status = value; invalid_status = null; } else invalid_status = value; }
        else if (name === "summary") summary = value;
        else if (name === "ref") { /* ignored (KR-SYN-7) */ }
        else if (name in coords) { const n = value === "" ? NaN : Number(value); coords[name] = Number.isFinite(n) ? n : null; }
        else extras.push(trimmed);
      }
    }
  }
  return { path: filePath, key, status, invalid_status, summary, refs: extractMarkers(body), body, folder_id: folderIdOf(filePath), ...coords, extras, bodyStartLine: declLines ? source.slice(0, declLines).split("\n").length : 1 };
}

// ---------------------------------------------------------------- fsref.rs

export class Fs {
  constructor(workspace) { this.ws = workspace; this.dirs = new Map(); this.rootCanon = undefined; this.texts = new Map(); }
  entry(dir, name) {
    if (!this.dirs.has(dir)) {
      let listing = null;
      try { listing = new Map(fs.readdirSync(dir, { withFileTypes: true }).map((d) => [d.name, d.isSymbolicLink()])); } catch { listing = null; }
      this.dirs.set(dir, listing);
    }
    const l = this.dirs.get(dir);
    return l && l.has(name) ? l.get(name) : undefined;
  }
  /** -> {kind: missing|escape|dir|file, len} */
  lookup(rel) {
    if (!isWorkspaceFileRef(rel)) return { kind: "missing", len: 0 };
    rel = rel.replace(/\\/g, "/");
    let cur = this.ws, linked = false;
    for (const seg of rel.split("/")) {
      const e = this.entry(cur, seg);
      if (e === undefined) return { kind: "missing", len: 0 };
      linked = linked || e;
      cur = path.join(cur, seg);
    }
    let st;
    try { st = fs.statSync(cur); } catch { return { kind: "missing", len: 0 }; }
    if (linked) {
      if (this.rootCanon === undefined) { try { this.rootCanon = fs.realpathSync(this.ws); } catch { this.rootCanon = null; } }
      let target = null;
      try { target = fs.realpathSync(cur); } catch { target = null; }
      const root = this.rootCanon;
      const inside = root && target && (target === root || target.startsWith(root.endsWith(path.sep) ? root : root + path.sep));
      if (!inside) return { kind: "escape", len: 0 };
    }
    return { kind: st.isDirectory() ? "dir" : "file", len: st.size };
  }
  /** -> {kind: missing|escape|unreadable|text, reason?, text?} */
  readText(rel) {
    if (this.texts.has(rel)) return this.texts.get(rel);
    const st = this.lookup(rel);
    let r;
    if (st.kind === "missing") r = { kind: "missing" };
    else if (st.kind === "escape") r = { kind: "escape" };
    else if (st.kind === "dir") r = { kind: "unreadable", reason: "directory" };
    else if (st.len > MAX_TEXT_BYTES) r = { kind: "unreadable", reason: "larger than 10 MiB" };
    else {
      let buf = null;
      try { buf = fs.readFileSync(path.join(this.ws, rel.replace(/\\/g, "/"))); } catch { buf = null; }
      if (!buf) r = { kind: "missing" };
      else if (buf.length > MAX_TEXT_BYTES) r = { kind: "unreadable", reason: "larger than 10 MiB" };
      else {
        try { r = { kind: "text", text: new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(buf) }; }
        catch { r = { kind: "unreadable", reason: "not UTF-8 text" }; }
      }
    }
    this.texts.set(rel, r);
    return r;
  }
}
export function lineCount(text) { // str::lines().count()
  if (text === "") return 0;
  const parts = text.split("\n").length;
  return text.endsWith("\n") ? parts - 1 : parts;
}
function bodyLines(text, start, end) { // TextEntry::body
  const lines = text.split(/(?<=\n)/);
  const out = [];
  for (let n = start; n <= end; n++) {
    const raw = lines[n - 1];
    if (raw === undefined) break;
    let l = raw.replace(/\n$/, "");
    if (l.endsWith("\r")) l = l.slice(0, -1);
    out.push(l.replace(/[\t\n\u000b\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+$/, ""));
  }
  return out.join("\n");
}

// ---------------------------------------------------------------- HEAD (git CLI)

export class Head {
  constructor(workspace, env = process.env) {
    this.ws = workspace; this.env = { ...env, GIT_LITERAL_PATHSPECS: "1", GIT_OPTIONAL_LOCKS: "0" };
    this.top = null; this.prefix = ""; this.commit = null; this.paths = new Map(); this.blobs = new Map();
    const g = (args, cwd = workspace) => spawnSync("git", args, { cwd, env: this.env, encoding: "utf8", maxBuffer: 256 << 20 });
    this.g = g;
    const r = g(["rev-parse", "--is-bare-repository", "--show-toplevel"]);
    if (r.status !== 0) return;
    const [bare, top] = r.stdout.trim().split("\n");
    if (bare === "true" || !top) return;
    let wsCanon, topCanon;
    try { wsCanon = fs.realpathSync(workspace); topCanon = fs.realpathSync(top); } catch { return; }
    if (wsCanon !== topCanon && !wsCanon.startsWith(topCanon + path.sep)) return;
    let prefix = path.relative(topCanon, wsCanon).split(path.sep).join("/");
    if (prefix && !prefix.endsWith("/")) prefix += "/";
    const c = g(["rev-parse", "--verify", "-q", "HEAD^{commit}"], topCanon);
    this.top = topCanon; this.prefix = prefix;
    this.commit = c.status === 0 ? c.stdout.trim() : null;
  }
  get on() { return this.top !== null && this.commit !== null; }
  /** HeadKey: "off" | "absent" | {oid} */
  key(rel) {
    if (!this.on) return "off";
    if (this.paths.has(rel)) return this.paths.get(rel);
    const full = this.prefix + rel;
    const r = this.g(["ls-tree", "-z", this.commit, "--", full], this.top);
    let k = "absent";
    if (r.status === 0) {
      for (const rec of r.stdout.split("\0")) {
        const tab = rec.indexOf("\t");
        if (tab < 0) continue;
        const [, type, oid] = rec.slice(0, tab).split(" ");
        if (rec.slice(tab + 1) === full && type === "blob") k = { oid };
      }
    } else k = "off";
    this.paths.set(rel, k);
    return k;
  }
  text(oid) {
    if (this.blobs.has(oid)) return this.blobs.get(oid);
    const r = spawnSync("git", ["cat-file", "blob", oid], { cwd: this.top, env: this.env, maxBuffer: 64 << 20 });
    const t = r.status === 0 ? new TextDecoder("utf-8", { ignoreBOM: true }).decode(r.stdout) : null;
    this.blobs.set(oid, t);
    return t;
  }
}

// ---------------------------------------------------------------- facts

class Facts {
  constructor(workspace, opts = {}) {
    this.fs = new Fs(workspace);
    this.head = new Head(workspace, opts.gitEnv || process.env);
    this.scopeCache = new Map();
    this.symbolCache = new Map();
  }
  file(p) { return this.fs.lookup(p).kind; }
  text(p) {
    const r = this.fs.readText(p);
    if (r.kind === "text") return { kind: "text", line_count: lineCount(r.text) };
    if (r.kind === "unreadable") return r;
    return { kind: "unreadable", reason: "not UTF-8 text" };
  }
  scopes(p, text, tag) {
    const k = tag + "\0" + p;
    if (!this.scopeCache.has(k)) this.scopeCache.set(k, supportsScopes(p) ? listScopes(p, text) : null);
    return this.scopeCache.get(k);
  }
  /** -> {fact: unsupported|missing|ambiguous|present|drifted, approx, start_line, end_line} */
  symbol(p, chain) {
    const ck = p + "\0" + chain;
    if (this.symbolCache.has(ck)) return this.symbolCache.get(ck);
    const r = this.fs.readText(p);
    const lang = languageOf(p);
    const approx = !!(lang && lang.approx);
    let out;
    if (r.kind !== "text") out = { fact: "missing", approx };
    else {
      const scopes = this.scopes(p, r.text, "disk");
      if (scopes === null) out = { fact: "unsupported", approx };
      else {
        const mm = matchScopes(scopes, chain);
        if (mm.kind === "missing") out = { fact: "missing", approx };
        else if (mm.kind === "ambiguous") out = { fact: "ambiguous", approx, hits: mm.hits.length };
        else {
          const e = mm.entry;
          const now = bodyLines(r.text, e.start_line, e.end_line);
          const hk = this.head.key(p);
          let drifted = null;
          if (hk === "absent") drifted = true;
          else if (hk !== "off") {
            const then = this.head.text(hk.oid);
            if (then === null) drifted = true;
            else {
              const hs = this.scopes(p, then, "head:" + hk.oid);
              const hm = hs ? matchScopes(hs, chain) : { kind: "missing" };
              drifted = hm.kind === "unique" ? bodyLines(then, hm.entry.start_line, hm.entry.end_line) !== now : true;
            }
          }
          out = { fact: drifted === true ? "drifted" : "present", approx, start_line: e.start_line, end_line: e.end_line, drifted };
        }
      }
    }
    this.symbolCache.set(ck, out);
    return out;
  }
}

// ---------------------------------------------------------------- validate.rs

const sevRank = (r) => (r.severity === "error" ? 2 : r.severity === "warning" ? 1 : 0);
function verdict(status, severity, message, extra = {}) { return { status, severity, message, extra }; }

function resolvePathCite(c, p, facts) {
  const file = isWorkspaceFileRef(p) ? facts.file(p) : "missing";
  if (file === "missing") return verdict("missing_file", "error", `File "${p}" does not exist.`);
  if (file === "escape") return verdict("symlink_escape", "error", `"${p}" is a link that resolves outside the workspace.`);
  if (c.bad_lines !== null) {
    if (c.kind === "symbol") return verdict("invalid_lines", "error", `Symbol citations do not take lines (lines="${c.bad_lines}"). Remove lines, or cite a line range as [@ file="${p}" lines="A-B"].`, { reason: INVALID_LINES_ON_SYMBOL });
    return verdict("invalid_lines", "error", `Lines "${c.bad_lines}" are not valid. Write lines="A" or lines="A-B" (decimal, 1 <= A <= B).`, { reason: INVALID_LINES_FORMAT });
  }
  if (c.kind === "file") return verdict("ok", null, "");
  const t = facts.text(p);
  if (t.kind === "unreadable") return verdict("unreadable_file", "error", `"${p}" is not a readable text file (${t.reason}).`, { reason: t.reason });
  if (c.symbol !== null) {
    const s = facts.symbol(p, c.symbol);
    const ex = s.approx ? { approx: true } : {};
    switch (s.fact) {
      case "unsupported": return verdict("unsupported_language", "error", `该语言不支持符号解析 ("${p}").`);
      case "missing": return verdict("missing_symbol", "error", `Symbol "${c.symbol}" in "${p}" does not exist.`, ex);
      case "ambiguous": return verdict("ambiguous_symbol", "error", `Symbol "${c.symbol}" in "${p}" is not unique.`, ex);
      case "drifted": return verdict("symbol_drift", "warning", `Symbol "${c.symbol}" in "${p}" differs from HEAD.`, { ...ex, start_line: s.start_line, end_line: s.end_line });
      default: return verdict("ok", null, "", { ...ex, start_line: s.start_line, end_line: s.end_line });
    }
  }
  if (!c.lines) return verdict("ok", null, "");
  if (c.lines.end > t.line_count) return verdict("line_out_of_range", "error", `Lines ${formatLineSpan(c.lines)} are outside "${p}", which has ${t.line_count} lines.`, { line_count: t.line_count });
  return verdict("ok", null, "");
}

function resolveCite(c, rk, node, ownKey, byKey, facts) {
  const p = normPath(c);
  const linesText = c.lines ? formatLineSpan(c.lines) : c.bad_lines;
  const args = {};
  let target_path = null, v;
  if (c.kind === "node") {
    const key = c.key ?? "";
    args.key = key;
    if (key === ownKey) v = verdict("self_relation", "error", "Citation points at itself.");
    else if (!byKey.has(key)) v = verdict("dangling_relation", "error", `Citation "${key}" does not exist.`);
    else {
      const t = byKey.get(key);
      target_path = t.path; args.target_status = t.status;
      v = node.status === "enabled" && t.status !== "enabled" ? verdict("inactive_target", "warning", `Cites ${t.status} "${key}".`) : verdict("ok", null, "");
    }
  } else {
    args.path = p;
    if (c.symbol !== null) args.symbol = c.symbol;
    if (linesText !== null && linesText !== undefined) args.lines = linesText;
    target_path = p;
    v = resolvePathCite(c, p, facts);
  }
  const ex = { ...v.extra };
  const meta = {};
  for (const k of ["approx", "start_line", "end_line"]) if (k in ex) { meta[k] = ex[k]; delete ex[k]; }
  return {
    ref_key: rk, kind: c.kind, key: c.key, path: p, symbol: c.symbol, lines: c.kind === "range" ? linesText : null,
    raw: c.raw, status: v.status, severity: v.severity, target_path, args: { ...args, ...ex }, message: v.message, meta,
    lineNo: c.lineNo,
  };
}

function declarationIssues(node, key, keyCounts) {
  const out = [];
  const push = (severity, code, message, args = {}) => out.push({ severity, code, message, ref_key: null, ref: args.key ?? args.status ?? null, args });
  if (node.invalid_status !== null) push("error", "invalid_status", `Status "${node.invalid_status}" is not valid. Use enabled, disabled, or pending.`, { status: node.invalid_status });
  if (key === "") push("error", "empty_key", "Declaration is missing.");
  else if (!isKnowledgeKey(key)) push("error", "empty_key", `Key "${key}" is not valid.`, { key });
  else if ((keyCounts.get(key) || 0) > 1) push("error", "duplicate_key", `Key "${key}" is duplicated.`, { key });
  else if (node.path && fileStem(node.path) !== key) { const stem = fileStem(node.path); push("warning", "filename_mismatch", `Filename "${stem}" does not match declaration "${key}".`, { key, filename: stem }); }
  return out;
}

export function resolve(nodes, facts) {
  const keyCounts = new Map();
  for (const n of nodes) { const k = normalizeKey(n.key); if (isKnowledgeKey(k)) keyCounts.set(k, (keyCounts.get(k) || 0) + 1); }
  const byKey = new Map();
  for (const n of nodes) { const k = normalizeKey(n.key); if (isKnowledgeKey(k) && !byKey.has(k)) byKey.set(k, { status: n.status, path: n.path }); }
  for (const n of nodes) {
    const key = normalizeKey(n.key);
    const issues = declarationIssues(n, key, keyCounts);
    const refs = [];
    const slot = new Map();
    for (const c of n.cites) {
      const rk = refKey(c);
      const r = resolveCite(c, rk, n, key, byKey, facts);
      if (slot.has(rk)) { const at = slot.get(rk); if (sevRank(r) > sevRank(refs[at])) refs[at] = r; }
      else { slot.set(rk, refs.length); refs.push(r); }
    }
    for (const r of refs) if (r.severity) issues.push({ severity: r.severity, code: r.status, message: r.message, ref_key: r.ref_key, args: r.args, meta: r.meta, lineNo: r.lineNo });
    n.issues = issues; n.refs = refs;
    n.severity = issues.some((i) => i.severity === "error") ? "error" : issues.some((i) => i.severity === "warning" && i.code !== "inactive_target") ? "warning" : null;
  }
  return nodes;
}

// ---------------------------------------------------------------- corpus / index

const cpCompare = (a, b) => { // Rust String / OsString order (UTF-8 bytes == code points)
  const A = Array.from(a), B = Array.from(b);
  for (let i = 0; i < Math.min(A.length, B.length); i++) { const x = A[i].codePointAt(0), y = B[i].codePointAt(0); if (x !== y) return x - y; }
  return A.length - B.length;
};

export function locateRoot(workspace) {
  const isDir = (p) => { try { return fs.statSync(p).isDirectory(); } catch { return false; } };
  if (isDir(path.join(workspace, "knowledge"))) return "knowledge";
  if (isDir(path.join(workspace, ".litecode/knowledge"))) return ".litecode/knowledge";
  return null;
}

function walk(abs, rel, files, dirs) {
  let entries;
  try { entries = fs.readdirSync(abs, { withFileTypes: true }); } catch { return; }
  entries.sort((a, b) => cpCompare(a.name, b.name));
  for (const e of entries) {
    if (e.name.startsWith(".")) continue;
    const child = rel ? `${rel}/${e.name}` : e.name;
    if (e.isDirectory()) { dirs.push(child); walk(path.join(abs, e.name), child, files, dirs); continue; }
    if (!e.name.toLowerCase().endsWith(".md")) continue;
    let md;
    try { md = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(fs.readFileSync(path.join(abs, e.name))); } catch { continue; }
    files.push(parseKnowledgeMarkdown(child, md));
  }
}

function addFolderChain(ids, dir) {
  let cur = dir;
  while (cur) { ids.add(cur); const s = cur.lastIndexOf("/"); cur = s < 0 ? "" : cur.slice(0, s); }
}

/** index::compute_once equivalent. */
export function computeOnce(workspace, opts = {}) {
  const root = locateRoot(workspace);
  if (!root) return { root: null, nodes: [], unknown: [], folders: [], head: null };
  const files = [], dirs = [];
  walk(path.join(workspace, root), "", files, dirs);
  files.sort((a, b) => cpCompare(a.path, b.path));
  const unknown = [], declared = [];
  for (const f of files) (normalizeKey(f.key) === "" ? unknown : declared).push(f);
  const counts = new Map();
  for (const f of declared) { const k = normalizeKey(f.key); if (isKnowledgeKey(k)) counts.set(k, (counts.get(k) || 0) + 1); }
  const used = new Set();
  const nodes = declared.map((f) => {
    const key = normalizeKey(f.key);
    let id = isKnowledgeKey(key) && counts.get(key) === 1 ? key : f.path;
    if (used.has(id)) id = f.path;
    used.add(id);
    return { ...f, id, key, value: f.body, relations: f.refs, cites: extractCites(f.body, true) };
  });
  const facts = new Facts(workspace, opts);
  resolve(nodes, facts);
  const folderIds = new Set();
  for (const d of dirs) addFolderChain(folderIds, d);
  for (const n of nodes) if (n.folder_id) addFolderChain(folderIds, n.folder_id);
  const unknownViews = unknown.map((u) => {
    const s = u.path.lastIndexOf("/");
    const folder_id = s < 0 ? null : u.path.slice(0, s);
    if (folder_id) addFolderChain(folderIds, folder_id);
    return { path: u.path, name: u.path.split("/").pop(), folder_id };
  });
  const folders = [...folderIds].map((id) => { const s = id.lastIndexOf("/"); return { id, name: s < 0 ? id : id.slice(s + 1), parent_id: s < 0 ? null : id.slice(0, s) }; }).sort((a, b) => cpCompare(a.id, b.id));
  return { root, nodes, unknown: unknownViews, folders, head: facts.head.commit, facts };
}
