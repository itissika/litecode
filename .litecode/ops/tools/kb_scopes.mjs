// ops.kb_check helper: named-scope listing without tree-sitter (no npm deps).
// Mirrors src/engines/code_search/enclosing.rs (list_scopes / match_scopes):
// each scope is {chain, kind, name, start_line, end_line}; chain = "kind name › kind name".
// Exact for the shapes tree-sitter yields on Rust / Python / JS / TS (the
// languages this corpus cites); Go and C-family are best-effort (approx: true),
// and kb_check downgrades their missing/ambiguous verdicts to a warning.

const SEP = " \u203a ";

export function languageOf(path) {
  const i = path.lastIndexOf(".");
  const ext = (i >= 0 ? path.slice(i + 1) : path).toLowerCase();
  // Rust's `rsplit('.').next()` on a path without a dot is the whole path.
  switch (ext) {
    case "rs": return { lang: "rust", approx: false };
    case "py": return { lang: "python", approx: false };
    case "js": case "mjs": case "cjs": return { lang: "js", approx: false };
    case "jsx": return { lang: "js", approx: true };
    case "ts": return { lang: "ts", approx: false };
    case "tsx": return { lang: "ts", approx: true };
    case "go": return { lang: "go", approx: true };
    case "c": case "h": return { lang: "c", approx: true };
    case "cc": case "cpp": case "cxx": case "hpp": case "hh": case "hxx": return { lang: "cpp", approx: true };
    case "java": return { lang: "java", approx: true };
    case "cs": return { lang: "cs", approx: true };
    default: return null;
  }
}

export function supportsScopes(path) { return languageOf(path) !== null; }

/** Every named scope, document order. Unsupported language -> null. */
export function listScopes(path, content) {
  const l = languageOf(path);
  if (!l) return null;
  if (content.length === 0) return [];
  const src = content;
  switch (l.lang) {
    case "rust": return rustScopes(src);
    case "python": return pythonScopes(src);
    case "js": case "ts": return jsScopes(src, l.lang === "ts");
    case "go": return goScopes(src);
    default: return cFamilyScopes(src, l.lang);
  }
}

// ---------------------------------------------------------------- matching

export function matchScopes(scopes, chain) {
  chain = rtrim(chain);
  if (!chain) return { kind: "missing" };
  let hits = scopes.filter((e) => e.chain === chain);
  if (hits.length) return hitsOf(hits);
  const legacy = legacyImplType(chain);
  if (!legacy) return { kind: "missing" };
  hits = scopes.filter((e) => legacyTraitImpl(e.chain, legacy.ty, legacy.rest));
  return hitsOf(hits);
}
function hitsOf(h) {
  if (h.length === 0) return { kind: "missing" };
  if (h.length === 1) return { kind: "unique", entry: h[0] };
  return { kind: "ambiguous", hits: h };
}
function splitOnce(s, sep) { const i = s.indexOf(sep); return i < 0 ? [s, ""] : [s.slice(0, i), s.slice(i + sep.length)]; }
function legacyImplType(chain) {
  const [head, rest] = splitOnce(chain, SEP);
  const h = rtrim(head);
  if (!h.startsWith("impl ")) return null;
  const ty = rtrim(h.slice(5));
  if (!ty || ty.includes(" for ")) return null;
  return { ty, rest };
}
function legacyTraitImpl(chain, ty, rest) {
  const [head, entryRest] = splitOnce(chain, SEP);
  if (entryRest !== rest) return false;
  const h = rtrim(head);
  if (!h.startsWith("impl ")) return false;
  const body = h.slice(5);
  const at = body.lastIndexOf(" for ");
  if (at < 0) return false;
  return rtrim(body.slice(0, at)) !== "" && rtrim(body.slice(at + 5)) === ty;
}

// Rust `str::trim` (Unicode White_Space; not U+FEFF).
const WS = new Set([..."\t\n\u000b\f\r \u0085\u00a0\u1680\u2000\u2001\u2002\u2003\u2004\u2005\u2006\u2007\u2008\u2009\u200a\u2028\u2029\u202f\u205f\u3000"]);
export function rtrim(s) {
  let a = 0, b = s.length;
  while (a < b && WS.has(s[a])) a++;
  while (b > a && WS.has(s[b - 1])) b--;
  return s.slice(a, b);
}
function collapse(s) { return s.split(/[\t\n\u000b\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+/).filter(Boolean).join(" "); }
function simplify(t) { // simplify_type_text
  const first = collapse(t).split(" ")[0] || t;
  return first.replace(/^[{}]+|[{}]+$/g, "");
}

// ---------------------------------------------------------------- shared

function lineIndex(src) {
  const starts = [0];
  for (let i = 0; i < src.length; i++) if (src.charCodeAt(i) === 10) starts.push(i + 1);
  return (off) => { // 1-based line of offset
    let lo = 0, hi = starts.length - 1;
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (starts[mid] <= off) lo = mid; else hi = mid - 1; }
    return lo + 1;
  };
}
function matchBrackets(toks) {
  const m = new Array(toks.length).fill(-1);
  const st = [];
  const pair = { ")": "(", "]": "[", "}": "{" };
  for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    if (t.k !== "p") continue;
    if (t.v === "(" || t.v === "[" || t.v === "{") st.push(i);
    else if (t.v in pair) {
      let j = st.length - 1;
      while (j >= 0 && toks[st[j]].v !== pair[t.v]) j--;
      if (j < 0) continue;
      const o = st[j]; st.length = j;
      m[o] = i; m[i] = o;
    }
  }
  for (const o of st) m[o] = toks.length - 1; // unclosed: runs to EOF
  return m;
}
const isOpen = (t) => t && t.k === "p" && (t.v === "(" || t.v === "[" || t.v === "{");
const ID_START = /[\p{ID_Start}_$]/u;
const ID_RE = /[\p{ID_Start}_][\p{ID_Continue}]*/uy;
const ID_RE_JS = /[\p{ID_Start}_$][\p{ID_Continue}$\u200c\u200d]*/uy;

class Emitter {
  constructor(src) { this.src = src; this.line = lineIndex(src); this.out = []; this.stack = []; }
  pop(i) { while (this.stack.length && i > this.stack[this.stack.length - 1].end) this.stack.pop(); }
  // start/end: char offsets (end exclusive); endTok index for stack
  emit(kind, name, startOff, endOff, endTok, push = true, extra = {}) {
    name = rtrim(name || "");
    if (!name) return null;
    const seg = { kind, name, ...extra };
    const segs = [...this.stack.map((s) => s.seg), seg];
    const start_line = this.line(startOff);
    let end_line = this.line(Math.max(startOff, endOff - 1));
    if (end_line < start_line) end_line = start_line;
    const entry = { chain: segs.map((s) => `${s.kind} ${s.name}`).join(SEP), kind, name, start_line, end_line };
    this.out.push(entry);
    if (push) this.stack.push({ seg, end: endTok, entry });
    return entry;
  }
}

// ---------------------------------------------------------------- Rust

function lexRust(src) {
  const toks = [];
  const n = src.length;
  let i = 0;
  const RAW = /(?:b|c)?r(#*)"/y, STR = /(?:b|c)?"/y, NUM = /[0-9][0-9A-Za-z_]*(?:\.[0-9][0-9A-Za-z_]*)?/y;
  const P2 = new Set(["->", "=>", "::", "!="]);
  while (i < n) {
    const c = src[i];
    if (WS.has(c) || c === "\ufeff") { i++; continue; }
    if (c === "/" && src[i + 1] === "/") { while (i < n && src[i] !== "\n") i++; continue; }
    if (c === "/" && src[i + 1] === "*") {
      let d = 1; i += 2;
      while (i < n && d > 0) {
        if (src[i] === "/" && src[i + 1] === "*") { d++; i += 2; } else if (src[i] === "*" && src[i + 1] === "/") { d--; i += 2; } else i++;
      }
      continue;
    }
    RAW.lastIndex = i;
    let m = RAW.exec(src);
    if (m) {
      const close = '"' + m[1];
      let j = src.indexOf(close, i + m[0].length);
      j = j < 0 ? n : j + close.length;
      toks.push({ k: "s", v: "str", s: i, e: j }); i = j; continue;
    }
    STR.lastIndex = i;
    m = STR.exec(src);
    if (m) {
      let j = i + m[0].length;
      while (j < n && src[j] !== '"') j += src[j] === "\\" ? 2 : 1;
      j = Math.min(n, j + 1);
      toks.push({ k: "s", v: "str", s: i, e: j }); i = j; continue;
    }
    if (c === "b" && src[i + 1] === "'") {
      let j = src[i + 2] === "\\" ? src.indexOf("'", i + 4) : src.indexOf("'", i + 3);
      j = j < 0 ? n : j + 1;
      toks.push({ k: "s", v: "char", s: i, e: j }); i = j; continue;
    }
    if (c === "'") {
      if (src[i + 1] === "\\") {
        let j = src.indexOf("'", i + 3); j = j < 0 ? n : j + 1;
        toks.push({ k: "s", v: "char", s: i, e: j }); i = j; continue;
      }
      const cp = src.codePointAt(i + 1);
      const len = cp === undefined ? 0 : (cp > 0xffff ? 2 : 1);
      if (len && src[i + 1 + len] === "'") { toks.push({ k: "s", v: "char", s: i, e: i + 2 + len }); i += 2 + len; continue; }
      ID_RE.lastIndex = i + 1;
      const lm = ID_RE.exec(src);
      const j = lm ? i + 1 + lm[0].length : i + 1;
      toks.push({ k: "l", v: src.slice(i, j), s: i, e: j }); i = j; continue;
    }
    if (c === "r" && src[i + 1] === "#" && ID_START.test(src[i + 2] || "")) {
      ID_RE.lastIndex = i + 2;
      const lm = ID_RE.exec(src);
      const j = i + 2 + lm[0].length;
      toks.push({ k: "i", v: src.slice(i, j), s: i, e: j }); i = j; continue;
    }
    ID_RE.lastIndex = i;
    m = ID_RE.exec(src);
    if (m) { toks.push({ k: "i", v: m[0], s: i, e: i + m[0].length }); i += m[0].length; continue; }
    NUM.lastIndex = i;
    m = NUM.exec(src);
    if (m) { toks.push({ k: "n", v: m[0], s: i, e: i + m[0].length }); i += m[0].length; continue; }
    const two = src.slice(i, i + 2);
    if (P2.has(two)) { toks.push({ k: "p", v: two, s: i, e: i + 2 }); i += 2; continue; }
    const cp = src.codePointAt(i);
    const len = cp > 0xffff ? 2 : 1;
    toks.push({ k: "p", v: src.slice(i, i + len), s: i, e: i + len }); i += len;
  }
  return toks;
}

function skipAngles(toks, m, j) { // toks[j] is '<'; returns index after matching '>'
  let d = 0;
  for (; j < toks.length; j++) {
    const t = toks[j];
    if (isOpen(t) && (t.v !== "{" || d > 0)) { j = m[j]; continue; }
    if (t.v === "<") d++;
    else if (t.v === ">") { d--; if (d === 0) return j + 1; }
    else if (t.v === "{" || t.v === ";") return j;
  }
  return j;
}
// From j, find the '{' (or ';') that ends an item header, at bracket/angle depth 0.
function headerEnd(toks, m, j, stopWhere = false) {
  let ang = 0;
  for (; j < toks.length; j++) {
    const t = toks[j];
    if (t.k === "p" && (t.v === "(" || t.v === "[")) { j = m[j]; continue; }
    if (t.v === "<") ang++;
    else if (t.v === ">") ang = Math.max(0, ang - 1);
    else if (t.v === "{" && ang === 0) return j;
    else if (t.v === "{") { j = m[j]; continue; }
    else if (t.v === ";" && ang === 0) return j;
    else if (stopWhere && ang === 0 && t.k === "i" && t.v === "where") return j;
  }
  return toks.length;
}

function rustScopes(src) {
  const toks = lexRust(src);
  // Rust 2024 `unsafe extern "ABI" { .. }`: the tree-sitter-rust grammar litecode
  // links does not know it; its error recovery drops the block's `{`, so the
  // block's `}` closes the enclosing block. Mirror that (verified via /symbols).
  for (let i = 0; i + 2 < toks.length; i++) {
    if (toks[i].k === "i" && toks[i].v === "unsafe" && toks[i + 1].k === "i" && toks[i + 1].v === "extern") {
      const j = toks[i + 2].k === "s" ? i + 3 : i + 2;
      if (toks[j] && toks[j].k === "p" && toks[j].v === "{") toks[j] = { ...toks[j], k: "x" };
    }
  }
  const m = matchBrackets(toks);
  const E = new Emitter(src);
  const MODS = new Set(["pub", "async", "const", "unsafe", "extern", "default"]);
  let atItem = true, modStart = -1;
  const reset = (item) => { atItem = item; modStart = -1; };
  for (let i = 0; i < toks.length; i++) {
    E.pop(i);
    const t = toks[i];
    if (t.k === "p" && t.v === "#") {
      let j = i + 1;
      if (toks[j] && toks[j].v === "!") j++;
      if (toks[j] && toks[j].v === "[") { i = m[j]; continue; } // attribute: keeps atItem/modStart
    }
    if (t.k === "i") {
      const nx = toks[i + 1];
      if (nx && nx.k === "p" && nx.v === "!") {
        if (t.v === "macro_rules" && toks[i + 2] && toks[i + 2].k === "i" && isOpen(toks[i + 3])) {
          const close = m[i + 3];
          let end = close;
          if (toks[i + 3].v !== "{" && toks[end + 1] && toks[end + 1].v === ";") end++;
          E.emit("macro", toks[i + 2].v, toks[modStart >= 0 ? modStart : i].s, toks[end].e, end, false);
          i = end; reset(true); continue;
        }
        if (isOpen(toks[i + 2])) { const brace = toks[i + 2].v === "{"; i = m[i + 2]; reset(brace); continue; }
      }
      if (atItem && MODS.has(t.v)) {
        if (modStart < 0) modStart = i;
        if (t.v === "pub" && toks[i + 1] && toks[i + 1].v === "(") i = m[i + 1];
        else if (t.v === "extern" && toks[i + 1] && toks[i + 1].k === "s") i++;
        continue;
      }
      const start = toks[modStart >= 0 ? modStart : i].s;
      if (t.v === "fn" && toks[i + 1] && toks[i + 1].k === "i") {
        const name = toks[i + 1].v;
        let j = i + 2;
        if (toks[j] && toks[j].v === "<") j = skipAngles(toks, m, j);
        if (toks[j] && toks[j].v === "(") j = m[j] + 1;
        j = headerEnd(toks, m, j);
        if (toks[j] && toks[j].v === "{") {
          E.emit("fn", name, start, toks[m[j]].e, m[j]);
          i = j; reset(true); continue;
        }
        i = Math.min(j, toks.length - 1); reset(true); continue;
      }
      if (t.v === "impl" && atItem) {
        let j = i + 1;
        if (toks[j] && toks[j].v === "<") j = skipAngles(toks, m, j);
        const hs = j;
        const he = headerEnd(toks, m, j, true);
        let forAt = -1, ang = 0;
        for (let k = hs; k < he; k++) {
          const v = toks[k].v;
          if (toks[k].k === "p" && (v === "(" || v === "[")) { k = m[k]; continue; }
          if (v === "<") ang++; else if (v === ">") ang = Math.max(0, ang - 1);
          else if (ang === 0 && toks[k].k === "i" && v === "for" && k > hs && !(toks[k + 1] && toks[k + 1].v === "<")) { forAt = k; break; }
        }
        const text = (a, b) => (a <= b ? collapse(src.slice(toks[a].s, toks[b].e)).replace(/\{+$/, "").trim() : "");
        let name;
        if (forAt >= 0) {
          let ta = hs; if (toks[ta].v === "!") ta++;
          const tr = text(ta, forAt - 1), ty = text(forAt + 1, he - 1);
          name = ty ? (tr ? `${tr} for ${ty}` : ty) : "";
        } else name = text(hs, he - 1);
        let b = he;
        if (toks[b] && toks[b].v === "where") b = headerEnd(toks, m, b);
        if (toks[b] && toks[b].v === "{") { E.emit("impl", name, start, toks[m[b]].e, m[b]); i = b; reset(true); continue; }
        i = Math.min(b, toks.length - 1); reset(true); continue;
      }
      if ((t.v === "struct" || t.v === "enum" || t.v === "trait" || t.v === "mod") && toks[i + 1] && toks[i + 1].k === "i") {
        const name = toks[i + 1].v;
        const j = headerEnd(toks, m, i + 2);
        if (toks[j] && toks[j].v === "{") { E.emit(t.v, name, start, toks[m[j]].e, m[j]); i = j; reset(true); continue; }
        if (toks[j] && toks[j].v === ";") { E.emit(t.v, name, start, toks[j].e, j, false); i = j; reset(true); continue; }
        i = Math.min(j, toks.length - 1); reset(true); continue;
      }
    }
    if (t.k === "p" && (t.v === "{" || t.v === "}" || t.v === ";")) { reset(true); continue; }
    reset(false);
  }
  return E.out;
}

// ---------------------------------------------------------------- Python

function pythonScopes(src) {
  const lines = src.split("\n");
  const n = lines.length;
  // Per physical line: starts inside a string / bracket / continuation?
  const inside = new Array(n).fill(false);
  const codeOf = new Array(n).fill(""); // code with strings blanked, comments removed
  let depth = 0, str = null, cont = false;
  for (let li = 0; li < n; li++) {
    inside[li] = depth > 0 || str !== null || cont;
    cont = false;
    const L = lines[li];
    let code = "";
    for (let i = 0; i < L.length; i++) {
      const c = L[i];
      if (str) {
        if (c === "\\") { i++; code += "  "; continue; }
        if (L.startsWith(str, i)) { i += str.length - 1; str = null; code += '"'; continue; }
        code += " ";
        continue;
      }
      if (c === "#") break;
      if (c === '"' || c === "'") {
        const tri = L.startsWith(c.repeat(3), i) ? c.repeat(3) : c;
        str = tri; i += tri.length - 1; code += '"'; continue;
      }
      if (c === "(" || c === "[" || c === "{") depth++;
      else if (c === ")" || c === "]" || c === "}") depth = Math.max(0, depth - 1);
      code += c;
    }
    if (str && str.length === 1) str = null; // unterminated single-quoted string ends at EOL
    if (/\\\s*$/.test(code) && !str) cont = true;
    codeOf[li] = code;
  }
  // Logical lines
  const logical = [];
  for (let li = 0; li < n; li++) {
    if (inside[li]) continue;
    if (!codeOf[li].trim() || /^[\s\ufeff]*$/.test(codeOf[li])) continue;
    let end = li;
    while (end + 1 < n && inside[end + 1]) end++;
    const indent = (lines[li].match(/^[ \t\ufeff]*/) || [""])[0].replace(/\ufeff/g, "").replace(/\t/g, "        ").length;
    logical.push({ start: li, end, indent, code: codeOf.slice(li, end + 1).join("\n") });
  }
  const out = [];
  const stack = []; // {seg, indent, endLine}
  const HEAD = /^[ \t\ufeff]*(?:(async)[ \t]+)?(def|class)[ \t]+([\p{ID_Start}_][\p{ID_Continue}]*)/u;
  for (let k = 0; k < logical.length; k++) {
    const L = logical[k];
    while (stack.length && L.start > stack[stack.length - 1].endLine) stack.pop();
    const mm = HEAD.exec(L.code);
    if (!mm) continue;
    const kind = mm[2] === "def" ? "fn" : "class";
    const name = mm[3];
    // inline body? text after the last depth-0 ':' of the header
    let d = 0, colon = -1;
    for (let i = 0; i < L.code.length; i++) {
      const c = L.code[i];
      if ("([{".includes(c)) d++; else if (")]}".includes(c)) d--; else if (c === ":" && d === 0) { colon = i; break; }
    }
    // the header colon is the first depth-0 ':' after the parameter list / class bases
    let endLine = L.end;
    const after = colon >= 0 ? L.code.slice(colon + 1).trim() : "";
    if (!after) {
      for (let q = k + 1; q < logical.length && logical[q].indent > L.indent; q++) endLine = logical[q].end;
    }
    const seg = { kind, name };
    const chain = [...stack.map((s) => s.seg), seg].map((s) => `${s.kind} ${s.name}`).join(SEP);
    out.push({ chain, kind, name, start_line: L.start + 1, end_line: endLine + 1 });
    stack.push({ seg, endLine });
  }
  return out;
}

// ---------------------------------------------------------------- JS / TS

const JS_KW_REGEX_PREV = new Set(["return", "typeof", "instanceof", "in", "of", "new", "delete", "void", "throw", "case", "do", "else", "yield", "await"]);
function lexJs(src) {
  const toks = [];
  const n = src.length;
  let i = 0;
  let nl = false;
  const push = (k, v, s, e) => { toks.push({ k, v, s, e, nl }); nl = false; };
  const skipString = (j, q) => { j++; while (j < n && src[j] !== q && src[j] !== "\n") j += src[j] === "\\" ? 2 : 1; return Math.min(n, j + 1); };
  const skipTemplate = (j) => { // src[j] === '`'
    j++;
    while (j < n) {
      const c = src[j];
      if (c === "\\") { j += 2; continue; }
      if (c === "`") return j + 1;
      if (c === "$" && src[j + 1] === "{") { j = skipCode(j + 2); continue; }
      j++;
    }
    return n;
  };
  const skipCode = (j) => { // until matching '}'
    let d = 1;
    while (j < n && d > 0) {
      const c = src[j];
      if (c === "/" && src[j + 1] === "/") { while (j < n && src[j] !== "\n") j++; continue; }
      if (c === "/" && src[j + 1] === "*") { const e = src.indexOf("*/", j + 2); j = e < 0 ? n : e + 2; continue; }
      if (c === '"' || c === "'") { j = skipString(j, c); continue; }
      if (c === "`") { j = skipTemplate(j); continue; }
      if (c === "{") d++; else if (c === "}") d--;
      j++;
    }
    return j;
  };
  const NUM = /[0-9.][0-9A-Za-z_.]*/y;
  const P3 = ["===", "!==", "...", "**=", "<<=", ">>=", "&&=", "||=", "??="];
  const P2 = new Set(["=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "**"]);
  while (i < n) {
    const c = src[i];
    if (c === "\n") { nl = true; i++; continue; }
    if (WS.has(c) || c === "\ufeff") { i++; continue; }
    if (c === "/" && src[i + 1] === "/") { while (i < n && src[i] !== "\n") i++; continue; }
    if (c === "/" && src[i + 1] === "*") { const e = src.indexOf("*/", i + 2); const j = e < 0 ? n : e + 2; if (src.slice(i, j).includes("\n")) nl = true; i = j; continue; }
    if (c === '"' || c === "'") { const j = skipString(i, c); push("s", "str", i, j); i = j; continue; }
    if (c === "`") { const j = skipTemplate(i); push("s", "tpl", i, j); i = j; continue; }
    if (c === "/") {
      const p = toks[toks.length - 1];
      const regexOk = !p || (p.k === "p" && !(p.v === ")" || p.v === "]" || p.v === "}")) || (p.k === "i" && JS_KW_REGEX_PREV.has(p.v));
      if (regexOk) {
        let j = i + 1, cls = false;
        while (j < n && src[j] !== "\n") {
          if (src[j] === "\\") { j += 2; continue; }
          if (src[j] === "[") cls = true; else if (src[j] === "]") cls = false; else if (src[j] === "/" && !cls) break;
          j++;
        }
        j++;
        while (j < n && /[a-z]/i.test(src[j])) j++;
        push("s", "re", i, j); i = j; continue;
      }
    }
    if (c === "#" && ID_START.test(src[i + 1] || "")) {
      ID_RE_JS.lastIndex = i + 1; const mm = ID_RE_JS.exec(src);
      push("i", "#" + mm[0], i, i + 1 + mm[0].length); i += 1 + mm[0].length; continue;
    }
    ID_RE_JS.lastIndex = i;
    let mm = ID_RE_JS.exec(src);
    if (mm) { push("i", mm[0], i, i + mm[0].length); i += mm[0].length; continue; }
    if (/[0-9]/.test(c) || (c === "." && /[0-9]/.test(src[i + 1] || ""))) { NUM.lastIndex = i; mm = NUM.exec(src); push("n", mm[0], i, i + mm[0].length); i += mm[0].length; continue; }
    const three = P3.find((p) => src.startsWith(p, i));
    if (three) { push("p", three, i, i + 3); i += 3; continue; }
    const two = src.slice(i, i + 2);
    if (P2.has(two)) { push("p", two, i, i + 2); i += 2; continue; }
    const cp = src.codePointAt(i); const len = cp > 0xffff ? 2 : 1;
    push("p", src.slice(i, i + len), i, i + len); i += len;
  }
  return toks;
}

const JS_NOT_METHOD = new Set(["if", "for", "while", "switch", "catch", "function", "with", "return", "typeof", "new", "await", "yield", "super", "import", "else", "do", "try", "finally", "case", "throw", "delete", "void", "in", "of", "instanceof"]);
const JS_MEMBER_MODS = new Set(["static", "async", "get", "set", "public", "private", "protected", "readonly", "override", "abstract", "declare", "accessor"]);
const JS_STMT_MODS = new Set(["export", "default", "declare", "async", "abstract"]);

// After ')' of params: skip an optional TS return type, return index of body '{' or ';' / other.
function jsBodyAfterParams(toks, m, j, ts) {
  if (!(toks[j] && toks[j].v === ":") || !ts) return j;
  j++;
  let expectType = true; let ang = 0;
  for (; j < toks.length; j++) {
    const t = toks[j];
    if (t.k === "p" && (t.v === "(" || t.v === "[")) { j = m[j]; expectType = false; continue; }
    if (t.v === "{") { if (expectType || ang > 0) { j = m[j]; expectType = false; continue; } return j; }
    if (t.v === "<") { ang++; expectType = true; continue; }
    if (t.v === ">") { ang = Math.max(0, ang - 1); expectType = false; continue; }
    if (t.v === ";" || t.v === "}" || t.v === "=") return j;
    if (t.v === "|" || t.v === "&" || t.v === "," || t.v === "=>" || t.v === ":" || t.v === "?" || t.v === "keyof" || t.v === "typeof" || t.v === "is" || t.v === "extends" || t.v === "infer") { expectType = true; continue; }
    expectType = false;
  }
  return j;
}

function jsScopes(src, ts) {
  const toks = lexJs(src);
  const m = matchBrackets(toks);
  const E = new Emitter(src);
  const ctx = []; // brace contexts: {close, kind}
  let stmt = true, modStart = -1;
  const completes = (p) => p && (p.k === "i" || p.k === "n" || p.k === "s" || (p.k === "p" && (p.v === ")" || p.v === "]" || p.v === "}" || p.v === "++" || p.v === "--")));
  for (let i = 0; i < toks.length; i++) {
    E.pop(i);
    while (ctx.length && i > ctx[ctx.length - 1].close) ctx.pop();
    const t = toks[i];
    const prev = toks[i - 1];
    if (!stmt && t.nl && completes(prev) && !(prev.k === "i" && JS_KW_REGEX_PREV.has(prev.v))) { stmt = true; modStart = -1; }
    const top = ctx[ctx.length - 1];
    const inClass = top && top.kind === "class" && i > top.open;
    if (t.k === "p" && t.v === "@" && toks[i + 1] && toks[i + 1].k === "i") { // decorator
      let j = i + 1;
      while (toks[j + 1] && toks[j + 1].v === "." && toks[j + 2] && toks[j + 2].k === "i") j += 2;
      if (toks[j + 1] && toks[j + 1].v === "(") j = m[j + 1];
      i = j; continue;
    }
    if (t.k === "i") {
      if (stmt && !inClass && JS_STMT_MODS.has(t.v) && toks[i + 1] && toks[i + 1].k === "i") {
        if (t.v === "async" || t.v === "abstract") { if (modStart < 0) modStart = i; } else modStart = -1;
        continue;
      }
      const start = toks[modStart >= 0 ? modStart : i].s;
      if (t.v === "function" && stmt) {
        let j = i + 1;
        if (toks[j] && toks[j].v === "*") j++;
        if (toks[j] && toks[j].k === "i") {
          const name = toks[j].v;
          let k = j + 1;
          if (toks[k] && toks[k].v === "<") k = skipAngles(toks, m, k);
          if (toks[k] && toks[k].v === "(") {
            const b = jsBodyAfterParams(toks, m, m[k] + 1, ts);
            if (toks[b] && toks[b].v === "{") { E.emit("fn", name, start, toks[m[b]].e, m[b]); ctx.push({ open: b, close: m[b], kind: "block" }); i = b; stmt = true; modStart = -1; continue; }
          }
        }
        stmt = false; modStart = -1; continue;
      }
      if (t.v === "class" && (stmt || (prev && prev.v === "abstract")) && toks[i + 1] && toks[i + 1].k === "i" && toks[i + 1].v !== "extends" && toks[i + 1].v !== "implements") {
        const name = toks[i + 1].v;
        let j = i + 2;
        for (; j < toks.length; j++) {
          if (toks[j].k === "p" && (toks[j].v === "(" || toks[j].v === "[")) { j = m[j]; continue; }
          if (toks[j].v === "<") { j = skipAngles(toks, m, j) - 1; continue; }
          if (toks[j].v === "{" || toks[j].v === ";") break;
        }
        if (toks[j] && toks[j].v === "{") { E.emit("class", name, start, toks[m[j]].e, m[j]); ctx.push({ open: j, close: m[j], kind: "class" }); i = j; stmt = true; modStart = -1; continue; }
      }
      if (ts && (t.v === "interface" || t.v === "enum") && stmt && toks[i + 1] && toks[i + 1].k === "i") {
        let j = i + 2;
        for (; j < toks.length; j++) {
          if (toks[j].k === "p" && (toks[j].v === "(" || toks[j].v === "[")) { j = m[j]; continue; }
          if (toks[j].v === "<") { j = skipAngles(toks, m, j) - 1; continue; }
          if (toks[j].v === "{" || toks[j].v === ";") break;
        }
        if (toks[j] && toks[j].v === "{") { E.emit(t.v, toks[i + 1].v, start, toks[m[j]].e, m[j]); ctx.push({ open: j, close: m[j], kind: t.v }); i = j; stmt = true; modStart = -1; continue; }
      }
      if (!ts && t.v === "enum") { /* not JS */ }
    }
    // class member or object-literal method
    const afterMod = prev && (prev.k === "i" && JS_MEMBER_MODS.has(prev.v) || prev.v === "*") && modStart >= 0;
    const memberStart = (inClass && (prev === undefined || prev.v === "{" || prev.v === ";" || prev.v === "}" || afterMod)) ||
      (!inClass && top && prev && (prev.v === "{" || prev.v === "," || afterMod));
    if (memberStart && (t.k === "i" || t.k === "s" || t.k === "n" || (t.k === "p" && (t.v === "[" || t.v === "*")))) {
      if (t.k === "i" && JS_MEMBER_MODS.has(t.v) && toks[i + 1] && (toks[i + 1].k === "i" || toks[i + 1].k === "s" || toks[i + 1].v === "[" || toks[i + 1].v === "*") && (inClass || ["async", "get", "set", "static"].includes(t.v))) {
        if (modStart < 0) modStart = i;
        continue;
      }
      let j = i;
      if (toks[j].v === "*") { if (modStart < 0) modStart = j; j++; }
      const nameTok = toks[j];
      if (!nameTok) continue;
      let name, after;
      if (nameTok.v === "[") { name = src.slice(nameTok.s, toks[m[j]].e); after = m[j] + 1; }
      else if (nameTok.k === "i" || nameTok.k === "s" || nameTok.k === "n") { name = src.slice(nameTok.s, nameTok.e); after = j + 1; }
      if (name !== undefined && !(nameTok.k === "i" && JS_NOT_METHOD.has(nameTok.v) && !inClass)) {
        let k = after;
        if (toks[k] && (toks[k].v === "?" || toks[k].v === "!")) k++;
        if (toks[k] && toks[k].v === "<") k = skipAngles(toks, m, k);
        if (toks[k] && toks[k].v === "(") {
          const b = jsBodyAfterParams(toks, m, m[k] + 1, ts);
          if (toks[b] && toks[b].v === "{") {
            const st = toks[modStart >= 0 ? modStart : i].s;
            E.emit("method", simplify(name), st, toks[m[b]].e, m[b]);
            ctx.push({ open: b, close: m[b], kind: "block" });
            i = b; stmt = true; modStart = -1; continue;
          }
        }
      }
    }
    if (t.k === "p" && t.v === "{") {
      ctx.push({ open: i, close: m[i], kind: "block" });
      stmt = true; modStart = -1; continue;
    }
    if (t.k === "p" && (t.v === "}" || t.v === ";")) { stmt = true; modStart = -1; continue; }
    stmt = false; modStart = -1;
  }
  return E.out;
}

// ---------------------------------------------------------------- Go (approx)

function lexC(src, opts = {}) {
  // C-like lexer: // and /* */ comments, "..." '...' strings; optional `raw` (Go backticks),
  // C# verbatim @"..", Java text blocks """...""".
  const toks = [];
  const n = src.length;
  let i = 0;
  while (i < n) {
    const c = src[i];
    if (WS.has(c) || c === "\ufeff") { i++; continue; }
    if (c === "/" && src[i + 1] === "/") { while (i < n && src[i] !== "\n") i++; continue; }
    if (c === "/" && src[i + 1] === "*") { const e = src.indexOf("*/", i + 2); i = e < 0 ? n : e + 2; continue; }
    if (opts.goRaw && c === "`") { const e = src.indexOf("`", i + 1); const j = e < 0 ? n : e + 1; toks.push({ k: "s", v: "str", s: i, e: j }); i = j; continue; }
    if (c === '"' && src.startsWith('"""', i)) { const e = src.indexOf('"""', i + 3); const j = e < 0 ? n : e + 3; toks.push({ k: "s", v: "str", s: i, e: j }); i = j; continue; }
    if (c === "@" && src[i + 1] === '"') { let j = i + 2; while (j < n) { if (src[j] === '"' && src[j + 1] === '"') { j += 2; continue; } if (src[j] === '"') break; j++; } j++; toks.push({ k: "s", v: "str", s: i, e: j }); i = j; continue; }
    if (c === '"' || c === "'") { let j = i + 1; while (j < n && src[j] !== c && src[j] !== "\n") j += src[j] === "\\" ? 2 : 1; j = Math.min(n, j + 1); toks.push({ k: "s", v: "str", s: i, e: j }); i = j; continue; }
    if (c === "#" && opts.cpp) { while (i < n && src[i] !== "\n") { if (src[i] === "\\" && src[i + 1] === "\n") i++; i++; } continue; } // preprocessor
    ID_RE.lastIndex = i;
    const mm = ID_RE.exec(src);
    if (mm) { toks.push({ k: "i", v: mm[0], s: i, e: i + mm[0].length }); i += mm[0].length; continue; }
    if (/[0-9]/.test(c)) { let j = i + 1; while (j < n && /[0-9A-Za-z_.']/.test(src[j])) j++; toks.push({ k: "n", v: src.slice(i, j), s: i, e: j }); i = j; continue; }
    const two = src.slice(i, i + 2);
    if (two === "::" || two === "->") { toks.push({ k: "p", v: two, s: i, e: i + 2 }); i += 2; continue; }
    toks.push({ k: "p", v: c, s: i, e: i + 1 }); i++;
  }
  return toks;
}

function goScopes(src) {
  const toks = lexC(src, { goRaw: true });
  const m = matchBrackets(toks);
  const E = new Emitter(src);
  const line = E.line;
  for (let i = 0; i < toks.length; i++) {
    E.pop(i);
    const t = toks[i];
    if (t.k !== "i") continue;
    if (t.v === "func") {
      let j = i + 1, kind = "fn", name;
      if (toks[j] && toks[j].v === "(") {
        const r = m[j] + 1;
        if (toks[r] && toks[r].k === "i" && toks[r + 1] && (toks[r + 1].v === "(" || toks[r + 1].v === "[")) { kind = "method"; name = toks[r].v; j = r + 1; } else continue;
      } else if (toks[j] && toks[j].k === "i") { name = toks[j].v; j++; } else continue;
      if (toks[j] && toks[j].v === "[") j = m[j] + 1;
      if (!(toks[j] && toks[j].v === "(")) continue;
      j = m[j] + 1;
      const l0 = line(toks[j - 1].s);
      for (; j < toks.length; j++) {
        if (toks[j].v === "(" || toks[j].v === "[") { j = m[j]; continue; }
        if (toks[j].v === "{" && !(toks[j - 1] && (toks[j - 1].v === "struct" || toks[j - 1].v === "interface"))) break;
        if (toks[j].v === "{") { j = m[j]; continue; }
        if (line(toks[j].s) !== l0 && toks[j].v !== "{") { j = -1; break; }
      }
      if (j < 0 || !toks[j]) { E.emit(kind, name, t.s, t.e, i, false); continue; }
      E.emit(kind, name, t.s, toks[m[j]].e, m[j]);
      i = j; continue;
    }
    if (t.v === "type") {
      const nx = toks[i + 1];
      if (nx && nx.v === "(") {
        const close = m[i + 1];
        const nm = toks[i + 2] && toks[i + 2].k === "i" ? toks[i + 2].v : "";
        E.emit("type", nm, t.s, toks[close].e, close, false);
        continue;
      }
      if (nx && nx.k === "i") {
        let j = i + 2, end = i + 1;
        const l0 = line(nx.s);
        for (; j < toks.length && line(toks[j].s) === l0; j++) {
          end = j;
          if (isOpen(toks[j])) { j = m[j]; end = j; break; }
        }
        E.emit("type", nx.v, t.s, toks[end].e, end, false);
      }
    }
  }
  return E.out;
}

// ---------------------------------------------------------------- C / C++ / Java / C# (approx)

const C_NOT_FN = new Set(["if", "for", "while", "switch", "catch", "return", "sizeof", "new", "using", "lock", "foreach", "fixed", "synchronized", "else", "do", "try", "throw", "await", "yield", "typeof", "nameof", "default", "case", "checked", "unchecked", "when", "alignof", "decltype", "static_assert", "defined"]);
const C_PREV_NOT_DECL = new Set(["return", "new", "throw", "await", "else", "case", "yield", "in", "is", "as", "goto", "sizeof", "typeof", "delete"]);

function cFamilyScopes(src, lang) {
  const cpp = lang === "c" || lang === "cpp";
  const toks = lexC(src, { cpp });
  const m = matchBrackets(toks);
  const E = new Emitter(src);
  const CONTAINERS = lang === "java" ? { class: "class", interface: "interface", enum: "enum", record: "record" }
    : lang === "cs" ? { class: "class", interface: "interface", enum: "enum", record: "record", struct: "struct", namespace: "namespace" }
    : lang === "cpp" ? { struct: "struct", class: "class", union: "union", enum: "enum", namespace: "namespace" }
    : { struct: "struct", union: "union", enum: "enum" };
  const classNames = [];
  for (let i = 0; i < toks.length; i++) {
    E.pop(i);
    while (classNames.length && i > classNames[classNames.length - 1].end) classNames.pop();
    const t = toks[i];
    const prev = toks[i - 1];
    if (t.k === "p" && t.v === "@" && toks[i + 1] && toks[i + 1].k === "i" && lang === "java") { let j = i + 1; while (toks[j + 1] && toks[j + 1].v === "." && toks[j + 2]) j += 2; if (toks[j + 1] && toks[j + 1].v === "(") j = m[j + 1]; i = j; continue; }
    if (t.k === "p" && t.v === "[" && lang === "cs" && prev && (prev.v === "{" || prev.v === "}" || prev.v === ";" || prev.v === "]")) { i = m[i]; continue; } // attributes
    if (t.k !== "i") continue;
    if (CONTAINERS[t.v] && !(prev && prev.v === ".")) {
      let j = i + 1;
      if (t.v === "enum" && toks[j] && (toks[j].v === "class" || toks[j].v === "struct")) j++;
      if (!(toks[j] && toks[j].k === "i")) continue;
      let ns = j, ne = j;
      while (toks[ne + 1] && (toks[ne + 1].v === "." || toks[ne + 1].v === "::") && toks[ne + 2] && toks[ne + 2].k === "i") ne += 2;
      const name = src.slice(toks[ns].s, toks[ne].e);
      let k = ne + 1;
      for (; k < toks.length; k++) {
        if (toks[k].v === "(" || toks[k].v === "[") { k = m[k]; continue; }
        if (toks[k].v === "{" || toks[k].v === ";" || toks[k].v === "=" || toks[k].v === ")" || toks[k].v === ",") break;
        if (cpp && toks[k].k === "i" && k === ne + 1 && !["final", "extends", "implements", "where", "sealed"].includes(toks[k].v)) break; // `struct stat st;`
      }
      const label = CONTAINERS[t.v];
      if (toks[k] && toks[k].v === "{") {
        E.emit(label, simplify(name), t.s, toks[m[k]].e, m[k]);
        if (label !== "namespace") classNames.push({ name: toks[ne].v, end: m[k] });
        i = k; continue;
      }
      if (label === "namespace" && toks[k] && toks[k].v === ";") { E.emit("namespace", simplify(name), t.s, toks[toks.length - 1].e, toks.length - 1); i = k; continue; }
      if (cpp) E.emit(label, simplify(name), t.s, toks[ne].e, ne, false);
      continue;
    }
    // function / method: name '(' ... ')' [quals] '{'   (java/cs also ';' inside a type body)
    if (toks[i + 1] && toks[i + 1].v === "(" && !C_NOT_FN.has(t.v)) {
      let nameStart = i;
      while (toks[nameStart - 1] && toks[nameStart - 1].v === "::" && toks[nameStart - 2] && toks[nameStart - 2].k === "i") nameStart -= 2;
      if (toks[nameStart - 1] && toks[nameStart - 1].v === "~") nameStart--;
      const p = toks[nameStart - 1];
      const declish = p && ((p.k === "i" && !C_PREV_NOT_DECL.has(p.v)) || p.v === ">" || p.v === "*" || p.v === "&" || p.v === "]" || p.v === "?");
      const inType = classNames.length > 0;
      const ctor = !p || ((lang === "java" || lang === "cs") && inType && t.v === classNames[classNames.length - 1].name);
      if (!declish && !ctor && !(cpp && (!p || p.v === "}" || p.v === ";"))) continue;
      let k = m[i + 1] + 1;
      for (; k < toks.length; k++) {
        if (toks[k].v === "(" || toks[k].v === "[") { k = m[k]; continue; }
        if (toks[k].v === "{" || toks[k].v === ";" || toks[k].v === "=" || toks[k].v === "," || toks[k].v === ")") break;
      }
      const label = cpp ? "fn" : (ctor ? "ctor" : "method");
      if (toks[k] && toks[k].v === "{") { E.emit(label, t.v, toks[nameStart].s, toks[m[k]].e, m[k]); i = k; continue; }
      if (!cpp && inType && toks[k] && toks[k].v === ";" && declish) { E.emit(label, t.v, toks[nameStart].s, toks[k].e, k, false); i = k; continue; }
    }
  }
  return E.out;
}
