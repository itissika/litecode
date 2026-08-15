import type { IDockviewPanelProps } from "dockview-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { MagnifyingGlass } from "@phosphor-icons/react";

import {
  getEnginesDetail,
  retrievalSearch,
  type RetrievalSearchHit,
  type SessionSearchHit,
} from "../../api/workspace";
import { getDockviewApi, useConnectionStore } from "../../stores/connectionStore";
import { useEditorStore } from "../../stores/editorStore";
import {
  fileBaseName,
  fileDir,
  SearchResultList,
  SearchSection,
} from "../../components/SearchResults";
import type { SearchResultGroup, SearchResultLine } from "../../components/SearchResults";

const TEXT_DEBOUNCE_MS = 280;
const SEMANTIC_DEBOUNCE_MS = 3000;
const WARM_POLL_MS = 5000;

type SearchTarget = "workspace" | "sessions";

function ToggleChip({
  label,
  title,
  active,
  onClick,
}: {
  label: string;
  title: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-pressed={active}
      onClick={onClick}
      className={`rounded px-1.5 py-0.5 font-mono text-dk-xs ${
        active
          ? "bg-(--_dk-ix-bg-selected) text-(--_dk-text-secondary)"
          : "text-(--_dk-text-muted) hover:text-(--_dk-text-secondary)"
      }`}
    >
      {label}
    </button>
  );
}

/** Group flat code hits by file → lines (sorted by line number). Mirrors VSCode. */
function buildCodeGroups(
  hits: RetrievalSearchHit[],
  onOpenFile: (hit: RetrievalSearchHit) => void,
  highlight: string,
  highlightCaseSensitive: boolean,
): SearchResultGroup[] {
  const byPath = new Map<string, SearchResultLine[]>();
  for (const hit of hits) {
    const lines = byPath.get(hit.path) ?? [];
    lines.push({
      id: `${hit.path}:${hit.start_line}`,
      lineLabel: String(hit.start_line),
      text: hit.summary,
      onOpen: () => onOpenFile(hit),
    });
    byPath.set(hit.path, lines);
  }
  return [...byPath.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([path, lines]) => {
      lines.sort((a, b) => Number(a.lineLabel) - Number(b.lineLabel));
      return {
        key: path,
        title: fileBaseName(path),
        subtitle: fileDir(path),
        lines,
        highlight,
        highlightCaseSensitive,
      };
    });
}

/** Group flat session hits by session → lines (sorted by seq). Mirrors VSCode. */
function buildSessionGroups(
  hits: SessionSearchHit[],
  highlight: string,
  highlightCaseSensitive: boolean,
): SearchResultGroup[] {
  const bySession = new Map<string, SearchResultLine[]>();
  for (const hit of hits) {
    const lines = bySession.get(hit.session_id) ?? [];
    lines.push({
      id: `${hit.session_id}:${hit.seq}`,
      lineLabel: String(hit.seq),
      text: hit.summary,
      onOpen: () => openAgentSession(hit.session_id),
    });
    bySession.set(hit.session_id, lines);
  }
  return [...bySession.entries()].map(([sid, lines]) => {
    lines.sort((a, b) => Number(a.lineLabel) - Number(b.lineLabel));
    return {
      key: sid,
      title: sid.slice(0, 8),
      subtitle: sid,
      lines,
      highlight,
      highlightCaseSensitive,
    };
  });
}

function openAgentSession(sessionId: string) {
  const api = getDockviewApi();
  if (!api) return;
  const panelId = `agent-${sessionId}`;
  if (api.getPanel(panelId)) {
    api.getPanel(panelId)?.api.setActive();
    void useConnectionStore.getState().ensureSubscribe(sessionId).catch(() => {});
    return;
  }
  const gridGroups = api.groups.filter((g) => g.api.location.type === "grid");
  let position: { referenceGroup: string } | undefined;
  if (gridGroups.length === 0) {
    const group = api.addGroup();
    position = { referenceGroup: group.id };
  } else {
    position = { referenceGroup: gridGroups[0].api.id };
  }
  api.addPanel({
    id: panelId,
    component: "agent",
    title: sessionId.slice(0, 8),
    params: { sessionId },
    tabComponent: "agent",
    position,
  });
}

export function SearchPanel(_props: IDockviewPanelProps) {
  const openFileAt = useEditorStore((s) => s.openFileAt);
  const [target, setTarget] = useState<SearchTarget>("workspace");
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [isRegex, setIsRegex] = useState(false);
  const [include, setInclude] = useState("");
  const [exclude, setExclude] = useState("");
  const [textHits, setTextHits] = useState<RetrievalSearchHit[]>([]);
  const [semanticHits, setSemanticHits] = useState<RetrievalSearchHit[]>([]);
  const [sessionHits, setSessionHits] = useState<SessionSearchHit[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [textBusy, setTextBusy] = useState(false);
  const [semBusy, setSemBusy] = useState(false);
  const [retrievalReady, setRetrievalReady] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const textDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const semDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const wasReadyRef = useRef(false);
  const textGenRef = useRef(0);
  const semGenRef = useRef(0);
  const sessionGenRef = useRef(0);

  const clearTimers = () => {
    if (textDebounceRef.current) clearTimeout(textDebounceRef.current);
    if (semDebounceRef.current) clearTimeout(semDebounceRef.current);
    textDebounceRef.current = null;
    semDebounceRef.current = null;
  };

  const refreshWarm = useCallback(async () => {
    try {
      const detail = await getEnginesDetail();
      setRetrievalReady(detail.retrieval.usable === "ready");
    } catch {
      setRetrievalReady(false);
    }
  }, []);

  useEffect(() => {
    void refreshWarm();
    const id = setInterval(() => {
      void refreshWarm();
    }, WARM_POLL_MS);
    return () => clearInterval(id);
  }, [refreshWarm]);

  const runTextSearch = useCallback(
    async (q: string) => {
      const trimmed = q.trim();
      const gen = ++textGenRef.current;
      if (!trimmed) {
        setTextHits([]);
        setError(null);
        return;
      }
      setTextBusy(true);
      setError(null);
      try {
        const data = await retrievalSearch({
          query: trimmed,
          corpus: "code",
          case_sensitive: caseSensitive,
          whole_word: wholeWord,
          is_regex: isRegex,
          include: include.trim() || undefined,
          exclude: exclude.trim() || undefined,
          top_k: 50,
          include_semantic: false,
        });
        if (gen !== textGenRef.current) return;
        setTextHits(data.text ?? []);
      } catch (e) {
        if (gen !== textGenRef.current) return;
        setTextHits([]);
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (gen === textGenRef.current) {
          setTextBusy(false);
        }
      }
    },
    [caseSensitive, wholeWord, isRegex, include, exclude],
  );

  const runSemanticSearch = useCallback(
    async (q: string) => {
      const trimmed = q.trim();
      const gen = ++semGenRef.current;
      if (!trimmed || !retrievalReady) {
        if (!trimmed && gen === semGenRef.current) setSemanticHits([]);
        return;
      }
      setSemBusy(true);
      try {
        const data = await retrievalSearch({
          query: trimmed,
          corpus: "code",
          case_sensitive: caseSensitive,
          include: include.trim() || undefined,
          top_k: 50,
          include_semantic: true,
        });
        if (gen !== semGenRef.current) return;
        // Keep previous semantic until a successful response (plan: retain until new returns).
        setSemanticHits(data.semantic ?? []);
      } catch {
        // Leave previous semantic hits on failure.
      } finally {
        if (gen === semGenRef.current) {
          setSemBusy(false);
        }
      }
    },
    [retrievalReady, caseSensitive, include],
  );

  const runSessionSearch = useCallback(
    async (q: string) => {
      const trimmed = q.trim();
      const gen = ++sessionGenRef.current;
      if (!trimmed) {
        setSessionHits([]);
        setError(null);
        return;
      }
      setTextBusy(true);
      setError(null);
      try {
        const data = await retrievalSearch({
          query: trimmed,
          corpus: "session",
          case_sensitive: caseSensitive,
          top_k: 50,
          // Session semantic column not wired in UI yet — skip ANN cost.
          include_semantic: false,
        });
        if (gen !== sessionGenRef.current) return;
        setSessionHits(data.session ?? []);
      } catch (e) {
        if (gen !== sessionGenRef.current) return;
        setSessionHits([]);
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (gen === sessionGenRef.current) {
          setTextBusy(false);
        }
      }
    },
    [caseSensitive],
  );

  // Workspace: dual debounce. Sessions: single text-like debounce.
  useEffect(() => {
    clearTimers();
    const trimmed = query.trim();
    if (!trimmed) {
      textGenRef.current += 1;
      semGenRef.current += 1;
      sessionGenRef.current += 1;
      setTextHits([]);
      setSemanticHits([]);
      setSessionHits([]);
      setError(null);
      setTextBusy(false);
      setSemBusy(false);
      return;
    }

    if (target === "sessions") {
      textDebounceRef.current = setTimeout(() => {
        void runSessionSearch(query);
      }, TEXT_DEBOUNCE_MS);
      return () => clearTimers();
    }

    textDebounceRef.current = setTimeout(() => {
      void runTextSearch(query);
    }, TEXT_DEBOUNCE_MS);

    if (retrievalReady) {
      semDebounceRef.current = setTimeout(() => {
        void runSemanticSearch(query);
      }, SEMANTIC_DEBOUNCE_MS);
    } else {
      setSemanticHits([]);
    }

    return () => clearTimers();
  }, [
    query,
    target,
    retrievalReady,
    runTextSearch,
    runSemanticSearch,
    runSessionSearch,
  ]);

  // Engine became ready while query is non-empty → schedule semantic (3s debounce).
  useEffect(() => {
    const becameReady = retrievalReady && !wasReadyRef.current;
    wasReadyRef.current = retrievalReady;
    if (!becameReady || target !== "workspace" || !query.trim()) return;
    if (semDebounceRef.current) clearTimeout(semDebounceRef.current);
    semDebounceRef.current = setTimeout(() => {
      void runSemanticSearch(query);
    }, SEMANTIC_DEBOUNCE_MS);
  }, [retrievalReady, target, query, runSemanticSearch]);

  useEffect(() => {
    const focus = () => inputRef.current?.focus();
    window.addEventListener("litecode:focus-workspace-search", focus);
    return () => window.removeEventListener("litecode:focus-workspace-search", focus);
  }, []);

  const switchTarget = (next: SearchTarget) => {
    if (next === target) return;
    clearTimers();
    setTarget(next);
    setTextHits([]);
    setSemanticHits([]);
    setSessionHits([]);
    setError(null);
  };

  const onOpenFile = (hit: RetrievalSearchHit) => {
    void openFileAt(hit.path, hit.start_line);
  };

  const showSemanticSection = target === "workspace" && retrievalReady;
  const busy = textBusy || semBusy;

  return (
    <div className="flex h-full flex-col bg-(--_dk-sidepanel)">
      <div className="shrink-0 space-y-2 border-b border-(--_dk-line) p-2">
        <div className="flex flex-wrap items-center gap-1">
          <ToggleChip
            label="Workspace"
            title="Search workspace files"
            active={target === "workspace"}
            onClick={() => switchTarget("workspace")}
          />
          <ToggleChip
            label="Sessions"
            title="Search conversation transcripts"
            active={target === "sessions"}
            onClick={() => switchTarget("sessions")}
          />
        </div>
        <div className="flex items-center gap-1 rounded border border-(--_dk-line) bg-(--_dk-editor) px-2">
          <MagnifyingGlass size={14} className="shrink-0 text-(--_dk-text-muted)" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={
              target === "workspace" ? "Search workspace" : "Search sessions"
            }
            className="min-w-0 flex-1 border-0 bg-transparent py-1.5 text-sm text-(--_dk-text-secondary) outline-none placeholder:text-(--_dk-text-disabled)"
          />
          {busy && (
            <span className="text-dk-2xs text-(--_dk-text-disabled)">…</span>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1">
          <ToggleChip
            label="Aa"
            title="Match case"
            active={caseSensitive}
            onClick={() => setCaseSensitive((v) => !v)}
          />
          {target === "workspace" && (
            <>
              <ToggleChip
                label="Ab"
                title="Match whole word"
                active={wholeWord}
                onClick={() => setWholeWord((v) => !v)}
              />
              <ToggleChip
                label=".*"
                title="Use regular expression"
                active={isRegex}
                onClick={() => setIsRegex((v) => !v)}
              />
            </>
          )}
        </div>
        {target === "workspace" && (
          <div className="grid grid-cols-1 gap-1">
            <input
              value={include}
              onChange={(e) => setInclude(e.target.value)}
              placeholder="files to include (e.g. *.rs, src/**)"
              className="w-full rounded border border-(--_dk-line) bg-(--_dk-editor) px-2 py-1 font-mono text-dk-xs text-(--_dk-text-secondary) outline-none placeholder:text-(--_dk-text-disabled)"
            />
            <input
              value={exclude}
              onChange={(e) => setExclude(e.target.value)}
              placeholder="files to exclude"
              className="w-full rounded border border-(--_dk-line) bg-(--_dk-editor) px-2 py-1 font-mono text-dk-xs text-(--_dk-text-secondary) outline-none placeholder:text-(--_dk-text-disabled)"
            />
          </div>
        )}
        {error && (
          <p className="text-xs text-(--_dk-red-500)">{error}</p>
        )}
      </div>

      <div className="flex min-h-0 flex-1 flex-col">
        {target === "workspace" ? (
          <>
            {showSemanticSection && (
              <SearchSection
                title="Semantic"
                count={semanticHits.length}
                empty={
                  query.trim()
                    ? semBusy
                      ? "Searching…"
                      : "No semantic matches"
                    : "Type to search"
                }
              >
                <SearchResultList groups={buildCodeGroups(semanticHits, onOpenFile, query.trim(), caseSensitive)} />
              </SearchSection>
            )}
            <SearchSection
              title="Text"
              count={textHits.length}
              empty={query.trim() ? "No text matches" : "Type to search"}
            >
              <SearchResultList groups={buildCodeGroups(textHits, onOpenFile, query.trim(), caseSensitive)} />
            </SearchSection>
          </>
        ) : (
          <SearchSection
            title="Sessions"
            count={sessionHits.length}
            empty={query.trim() ? "No session matches" : "Type to search"}
          >
            <SearchResultList groups={buildSessionGroups(sessionHits, query.trim(), caseSensitive)} />
          </SearchSection>
        )}
      </div>
    </div>
  );
}
