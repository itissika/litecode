/**
 * Parse the agent-facing `session_search` text.
 *
 * The only producer is `render_view` (`src/engines/session_search/mod.rs`).
 * A line that is not that grammar fails the whole parse so the card can show
 * the raw text instead of a half-drawn result.
 */

export interface SessionSearchHit {
  from: number;
  /** Set when the label is a range (`L2-4`). */
  to?: number;
  label: string;
  /** Body lines with the two-space indent removed. */
  lines: string[];
}

export interface SessionSearchGroup {
  handle: string;
  age: string;
  /** Header total for the session, not how many hits this card carries. */
  count: number;
  hits: SessionSearchHit[];
}

export interface SessionSearchFooter {
  shown: number;
  total: number;
  remaining: number;
  location: string;
  /** Text after `More in: `, without the trailing period. */
  more?: string;
}

export type SessionSearchView =
  | {
      kind: "hits";
      groups: SessionSearchGroup[];
      footer?: SessionSearchFooter;
    }
  | { kind: "empty"; text: string };

/** Middle dot used as the header separator (U+00B7). */
const DOT = "\u00b7";

const EMPTY_RE =
  /^No matching session transcript context for query '[\s\S]*'\.$/;

const HEADER_RE = new RegExp(
  `^### (\\S+) ${DOT} (.+) ${DOT} (\\d+) Matches$`,
);

const HIT_RE = /^L(\d+)(?:-(\d+))?: (.*)$/;

const SHOWING_RE = new RegExp(
  `^Showing (\\d+) of (\\d+) hits; the remaining (\\d+) are in (.*?) \u2014 read or grep it\\.$`,
);

const MORE_RE = /^More in: (\S+ \d+(?:, \S+ \d+)*)\.$/;

export function parseSessionSearch(raw: string): SessionSearchView | null {
  const normalized = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const body = normalized.endsWith("\n")
    ? normalized.slice(0, -1)
    : normalized;
  if (EMPTY_RE.test(body)) return { kind: "empty", text: body };
  if (body === "") return null;

  const lines = body.split("\n");
  const groups: SessionSearchGroup[] = [];
  let group: SessionSearchGroup | null = null;
  let hit: SessionSearchHit | null = null;

  const closeHit = (): boolean => {
    if (!hit || hit.lines.length === 0 || !group) return false;
    group.hits.push(hit);
    hit = null;
    return true;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line === "") {
      if (!closeHit()) return null;
      const footer = parseFooter(lines.slice(i + 1));
      if (!footer) return null;
      return { kind: "hits", groups, footer };
    }
    // Classify the raw line. Two leading spaces are body even when the rest
    // looks like a header or a hit.
    if (line.startsWith("  ")) {
      if (!hit) return null;
      hit.lines.push(line.slice(2));
      continue;
    }
    const header = HEADER_RE.exec(line);
    if (header) {
      if (hit && !closeHit()) return null;
      if (group && group.hits.length === 0) return null;
      group = {
        handle: header[1],
        age: header[2],
        count: Number(header[3]),
        hits: [],
      };
      groups.push(group);
      continue;
    }
    const hitMatch = HIT_RE.exec(line);
    if (hitMatch) {
      if (!group) return null;
      if (hit && !closeHit()) return null;
      hit = {
        from: Number(hitMatch[1]),
        label: hitMatch[3],
        lines: [],
      };
      if (hitMatch[2] !== undefined) hit.to = Number(hitMatch[2]);
      continue;
    }
    return null;
  }

  if (!closeHit()) return null;
  return { kind: "hits", groups };
}

function parseFooter(lines: string[]): SessionSearchFooter | null {
  if (lines.length < 1 || lines.length > 2) return null;
  const showing = SHOWING_RE.exec(lines[0]);
  if (!showing) return null;
  const footer: SessionSearchFooter = {
    shown: Number(showing[1]),
    total: Number(showing[2]),
    remaining: Number(showing[3]),
    location: showing[4],
  };
  if (lines.length === 2) {
    const more = MORE_RE.exec(lines[1]);
    if (!more) return null;
    footer.more = more[1];
  }
  return footer;
}
