import type { CSSProperties, ReactNode } from "react";

import { FoldCard, FOLDCARD_HEADER_TONE } from "./FoldCard";

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Split `text` into plain + matched spans, wrapping occurrences of `needle`
 *  (the search query) in `.search-hit` so they render in the amber highlight
 *  color. Matches are character-level, not background — case follows the flag. */
function highlightText(
  text: string,
  needle: string | undefined,
  caseSensitive: boolean,
): ReactNode {
  if (!needle) return text;
  let re: RegExp;
  try {
    re = new RegExp(escapeRegExp(needle), caseSensitive ? "g" : "gi");
  } catch {
    return text;
  }
  const parts: ReactNode[] = [];
  let last = 0;
  let key = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    parts.push(
      <span key={key++} className="search-hit">
        {m[0]}
      </span>,
    );
    last = m.index + m[0].length;
    if (m[0].length === 0) re.lastIndex++;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

/** Split a workspace-relative path on "/" into (base, dir). */
export function fileBaseName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

/** Directory portion of a workspace-relative path (empty string for a bare file). */
export function fileDir(path: string): string {
  const parts = path.split("/");
  parts.pop();
  return parts.join("/");
}

/** A single matched line under a result group. Clicking it jumps to the location. */
export interface SearchResultLine {
  /** Stable key within the group. */
  id: string;
  /** Position label shown before the line, e.g. line number "12" or seq "34". */
  lineLabel: string;
  /** The matched single line of text. */
  text: string;
  /** Jump to the code position / session position. */
  onOpen: () => void;
}

/** A primary key (file or session) and its matched lines. */
export interface SearchResultGroup {
  /** Stable key for the group (file path or session id). */
  key: string;
  /** Header primary label: file name or session title. */
  title: string;
  /** Header secondary label: path summary (dir) or session id. */
  subtitle?: string;
  /** Optional header icon. */
  icon?: ReactNode;
  /** Matched lines under this group. */
  lines: SearchResultLine[];
  /** Full-result match count from the server; falls back to `lines.length`. */
  matchCount?: number;
  /** Open the group target (file / session) without toggling fold. */
  onOpenTitle?: () => void;
  /** Query string to highlight inside each line (matched characters). */
  highlight?: string;
  /** Whether `highlight` matching is case-sensitive. */
  highlightCaseSensitive?: boolean;
}

export function SearchResultLineRow({
  line,
  highlight,
  caseSensitive,
}: {
  line: SearchResultLine;
  highlight?: string;
  caseSensitive?: boolean;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={line.onOpen}
        title={line.text}
        className="flex w-full items-baseline gap-2 px-1.5 py-1 text-left transition-colors hover:bg-(--_dk-ix-bg-hover)"
      >
        <span className="shrink-0 select-none font-mono text-dk-xs tabular-nums text-(--_dk-text-disabled)">
          {line.lineLabel}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-dk-xs text-(--_dk-text-secondary)">
          {highlightText(line.text, highlight, caseSensitive ?? false)}
        </span>
      </button>
    </li>
  );
}

export function SearchResultGroupCard({ group }: { group: SearchResultGroup }) {
  const count = group.matchCount ?? group.lines.length;
  const title = group.onOpenTitle ? (
    <button
      type="button"
      onClick={group.onOpenTitle}
      title={group.title}
      className="truncate font-mono text-dk-xs text-(--_dk-text-secondary) hover:text-(--_dk-text-primary)"
    >
      {group.title}
    </button>
  ) : (
    <span className="truncate font-mono text-dk-xs text-(--_dk-text-secondary)">
      {group.title}
    </span>
  );
  return (
    <FoldCard
      defaultOpen
      className="search-foldcard"
      icon={group.icon}
      label={
        <span className={`${FOLDCARD_HEADER_TONE} flex min-w-0 items-baseline gap-1.5`}>
          {title}
          {group.subtitle && (
            <span className="truncate font-mono text-dk-xs text-(--_dk-text-disabled)">
              {group.subtitle}
            </span>
          )}
          <span className="ml-auto shrink-0 pl-1 text-dk-2xs text-(--_dk-text-disabled)">
            {count}
          </span>
        </span>
      }
    >
      <ul>
        {group.lines.map((line) => (
          <SearchResultLineRow
            key={line.id}
            line={line}
            highlight={group.highlight}
            caseSensitive={group.highlightCaseSensitive}
          />
        ))}
      </ul>
    </FoldCard>
  );
}

export function SearchResultList({ groups }: { groups: SearchResultGroup[] }) {
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="py-1">
        {groups.map((g) => (
          <SearchResultGroupCard key={g.key} group={g} />
        ))}
      </div>
    </div>
  );
}

/** A labelled section (e.g. "Text" / "Semantic" / "Sessions") with an empty state. */
export function SearchSection({
  title,
  count,
  empty,
  children,
  style,
}: {
  title: string;
  count: number;
  empty: string;
  children: ReactNode;
  /** Overrides the default flex-1 sizing (e.g. a resizable split between sections). */
  style?: CSSProperties;
}) {
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col border-t border-(--_dk-line)"
      style={style}
    >
      <div className="shrink-0 px-2 py-1.5 text-dk-2xs uppercase tracking-wide text-(--_dk-text-muted)">
        {title}
        <span className="ml-1 text-(--_dk-text-disabled)">({count})</span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col">
        {count === 0 ? (
          <p className="px-2 py-2 text-xs text-(--_dk-text-disabled)">{empty}</p>
        ) : (
          children
        )}
      </div>
    </div>
  );
}
