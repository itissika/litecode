import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { Eye, FileText, PencilSimple } from "@phosphor-icons/react";

import { mentionKeysOf } from "../../lib/knowledge/flowProjection";
import { extractMarkers, normalizeKey } from "../../lib/knowledge/markers";
import type { KnowledgeIssue } from "../../lib/knowledge/types";
import { knowledgeTitleTone } from "../../lib/knowledge/validate";
import { useKnowledgeStore } from "../../stores/knowledgeStore";
import { AgentMarkdown } from "../AgentMarkdown";
import { CitationChip } from "../CitationChip";
import { KnowledgeBodyEditor } from "./KnowledgeBodyEditor";
import { SaveGlyph, type CardSaveMark } from "./KnowledgeSourceField";
import i18n from "../../i18n";

const NO_ISSUES: KnowledgeIssue[] = [];

/**
 * Default knowledge surface: read one node as a document. The body goes
 * through the same markdown renderer and citation chips as the rest of the
 * app, so a node chip changes this document instead of opening another panel.
 * The summary is always editable; the body uses the canvas card's edit toggle.
 * Saves run on blur, on Cmd/Ctrl+S, on leaving edit mode, and on unmount.
 */
export function KnowledgeDocument() {
  const focusedId = useKnowledgeStore((s) => s.focusedId);
  const node = useKnowledgeStore((s) =>
    s.focusedId ? s.byId.get(s.focusedId) : undefined,
  );
  const nodes = useKnowledgeStore((s) => s.nodes);
  const issues = useKnowledgeStore((s) =>
    s.focusedId ? (s.issuesByNode.get(s.focusedId) ?? NO_ISSUES) : NO_ISSUES,
  );
  const mentionKeys = useKnowledgeStore((s) => mentionKeysOf(s.nodes));
  const saveNode = useKnowledgeStore((s) => s.saveNode);
  const focusCanvas = useKnowledgeStore((s) => s.focusCanvas);
  const clearCanvasFocus = useKnowledgeStore((s) => s.clearCanvasFocus);
  const revealInList = useKnowledgeStore((s) => s.revealInList);

  const summaryId = useId();
  const nodeId = node?.id ?? null;
  const nodeIdRef = useRef(nodeId);
  nodeIdRef.current = nodeId;

  // A document surface needs a subject: when the panel opens without a focus,
  // read the first node rather than showing a dead pane. A stale id (node
  // deleted) falls back the same way.
  useEffect(() => {
    if (focusedId == null && nodes.length > 0) {
      focusCanvas(nodes[0].id);
    } else if (focusedId != null && !node) {
      if (nodes.length > 0) focusCanvas(nodes[0].id);
      else clearCanvasFocus();
    }
  }, [focusedId, node, nodes, focusCanvas, clearCanvasFocus]);

  // Reading a node should also locate it in the side list: expand its ancestor
  // folders and let the browser's flash-scroll effect bring the row into view.
  useEffect(() => {
    if (focusedId) revealInList(focusedId);
  }, [focusedId, revealInList]);

  const storedBody = (node?.value ?? "").replace(/\s+$/, "");
  const storedSummary = node?.summary ?? "";
  const [draft, setDraft] = useState(storedBody);
  const [summaryDraft, setSummaryDraft] = useState(storedSummary);
  const [editing, setEditing] = useState(false);
  const [saveMark, setSaveMark] = useState<CardSaveMark | null>(null);

  const draftRef = useRef(draft);
  const summaryDraftRef = useRef(summaryDraft);
  const bodyDirtyRef = useRef(false);
  const summaryDirtyRef = useRef(false);
  const mounted = useRef(true);
  const savedTimer = useRef<number | null>(null);
  const saveChain = useRef<Promise<boolean>>(Promise.resolve(true));
  draftRef.current = draft;
  summaryDraftRef.current = summaryDraft;

  const flashSaved = useCallback(() => {
    setSaveMark("saved");
    if (savedTimer.current != null) window.clearTimeout(savedTimer.current);
    savedTimer.current = window.setTimeout(() => {
      if (mounted.current) setSaveMark(null);
    }, 1200);
  }, []);

  const commitDrafts = useCallback((): Promise<boolean> => {
    const id = nodeIdRef.current;
    if (!id) return Promise.resolve(true);
    if (!bodyDirtyRef.current && !summaryDirtyRef.current) {
      return Promise.resolve(true);
    }
    // A rename or delete can remove the old id before unmount saves run.
    if (!useKnowledgeStore.getState().byId.has(id)) return Promise.resolve(true);
    if (mounted.current) {
      if (savedTimer.current != null) {
        window.clearTimeout(savedTimer.current);
        savedTimer.current = null;
      }
      setSaveMark("saving");
    }
    // Serialize writes: blur + preview click can both ask for a save, and a
    // second write must not overtake the first. Each run saves the newest
    // drafts so a keystroke during the request is not dropped.
    const run = saveChain.current.then(async () => {
      const latestBody = draftRef.current;
      const latestSummary = summaryDraftRef.current.trim();
      const patch: Parameters<typeof saveNode>[1] = {};
      if (bodyDirtyRef.current) {
        patch.value = latestBody;
        patch.relations = extractMarkers(latestBody);
      }
      if (summaryDirtyRef.current) patch.summary = latestSummary;
      if (Object.keys(patch).length === 0) {
        if (mounted.current) setSaveMark(null);
        return true;
      }
      try {
        const ok = await saveNode(id, patch);
        if (!mounted.current) return ok;
        if (!ok) {
          setSaveMark("dirty");
          return false;
        }
        let remaining = false;
        if (bodyDirtyRef.current) {
          if (draftRef.current === latestBody) bodyDirtyRef.current = false;
          else remaining = true;
        }
        if (summaryDirtyRef.current) {
          if (summaryDraftRef.current.trim() === latestSummary) {
            summaryDirtyRef.current = false;
          } else {
            remaining = true;
          }
        }
        if (remaining) setSaveMark("dirty");
        else flashSaved();
        return true;
      } catch {
        if (mounted.current) setSaveMark("dirty");
        return false;
      }
    });
    saveChain.current = run.catch(() => false);
    return run;
  }, [flashSaved, saveNode]);

  // Save whatever is still dirty when the reader leaves this node, switches to
  // the canvas, or closes the panel. State updates are guarded by `mounted`.
  const commitRef = useRef(commitDrafts);
  commitRef.current = commitDrafts;
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (savedTimer.current != null) window.clearTimeout(savedTimer.current);
      void commitRef.current();
    };
  }, []);

  useEffect(() => {
    if (bodyDirtyRef.current) return;
    draftRef.current = storedBody;
    setDraft(storedBody);
  }, [storedBody]);

  useEffect(() => {
    if (summaryDirtyRef.current) return;
    summaryDraftRef.current = storedSummary;
    setSummaryDraft(storedSummary);
  }, [storedSummary]);

  const ownKey = node ? normalizeKey(node.key) : "";
  const refCandidates = useMemo(
    () =>
      mentionKeys
        ? mentionKeys.split("\n").filter((key) => key && key !== ownKey)
        : [],
    [mentionKeys, ownKey],
  );

  const outgoing = useMemo(
    () => Array.from(new Set(node?.relations ?? [])),
    [node?.relations],
  );
  const incoming = useMemo(() => {
    if (!node) return [];
    return nodes.filter(
      (candidate) =>
        candidate.id !== node.id &&
        candidate.relations.some((key) => normalizeKey(key) === ownKey),
    );
  }, [node, nodes, ownKey]);

  function changeBody(next: string) {
    draftRef.current = next;
    setDraft(next);
    bodyDirtyRef.current = true;
    setSaveMark("dirty");
  }

  function changeSummary(next: string) {
    summaryDraftRef.current = next;
    setSummaryDraft(next);
    summaryDirtyRef.current = true;
    setSaveMark("dirty");
  }

  async function toggleEdit() {
    if (!editing) {
      setEditing(true);
      return;
    }
    if (await commitDrafts()) setEditing(false);
  }

  async function toggleStatus() {
    const id = node?.id;
    if (!id) return;
    if (!(await commitDrafts())) return;
    const current = useKnowledgeStore.getState().byId.get(id);
    if (!current) return;
    await saveNode(current.id, {
      status: current.status === "enabled" ? "disabled" : "enabled",
    });
  }

  function onDocumentKeyDown(event: KeyboardEvent<HTMLElement>) {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void commitDrafts();
    }
  }

  if (!node) {
    return (
      <div className="knowledge-document">
        <div className="knowledge-document-empty">
          <FileText size={28} weight="thin" aria-hidden />
          <p className="text-dk-sm">{i18n.t("knowledge.emptyTitle")}</p>
          <p className="text-dk-xs">{i18n.t("knowledge.emptyHint")}</p>
        </div>
      </div>
    );
  }

  const enabled = node.status === "enabled";
  const tone = knowledgeTitleTone(issues, node.status);
  // The switch already says disabled; a struck-through document title is noise.
  const titleTone = tone === "disabled" ? null : tone;

  return (
    <div className="knowledge-document">
      <article className="knowledge-document-article" onKeyDown={onDocumentKeyDown}>
        <header>
          <div className="knowledge-document-title-row">
            <h1
              className={`knowledge-node-title knowledge-document-title ${
                titleTone ? `is-${titleTone}` : ""
              }`}
            >
              {node.key}
            </h1>
            <button
              type="button"
              role="switch"
              aria-checked={enabled}
              aria-label={enabled ? i18n.t("common.disable") : i18n.t("common.enable")}
              title={enabled ? i18n.t("common.disable") : i18n.t("common.enable")}
              className={enabled ? "knowledge-enable is-on" : "knowledge-enable"}
              onClick={() => void toggleStatus()}
            />
          </div>
        </header>

        <div className="knowledge-document-summary">
          <div className="knowledge-document-summary-head">
            <label htmlFor={summaryId} className="knowledge-source-label">
              {i18n.t("common.summary")}
            </label>
            <SaveGlyph mark={saveMark} />
          </div>
          <textarea
            id={summaryId}
            className="knowledge-document-summary-input"
            rows={2}
            spellCheck={false}
            value={summaryDraft}
            placeholder={i18n.t("knowledge.summaryPlaceholder")}
            onChange={(event) =>
              changeSummary(event.target.value.replace(/[\r\n]+/g, " "))
            }
            onKeyDown={(event) => {
              if (event.key === "Enter") event.preventDefault();
            }}
            onBlur={() => void commitDrafts()}
          />
        </div>

        <div className="knowledge-document-body">
          <div className="knowledge-document-body-bar">
            <span className="knowledge-source-label">{i18n.t("common.body")}</span>
            <button
              type="button"
              className="knowledge-flow-body-edit nodrag nowheel"
              aria-pressed={editing}
              aria-label={editing ? i18n.t("knowledge.previewBody") : i18n.t("knowledge.editBody")}
              onClick={() => void toggleEdit()}
            >
              {editing ? (
                <Eye size={12} aria-hidden />
              ) : (
                <PencilSimple size={12} aria-hidden />
              )}
            </button>
          </div>
          {editing ? (
            <div className="knowledge-document-editor">
              <KnowledgeBodyEditor
                label={i18n.t("common.body")}
                sourceId={node.id}
                value={draft}
                candidates={refCandidates}
                rows={18}
                onChange={changeBody}
                onBlur={() => void commitDrafts()}
              />
            </div>
          ) : (
            <AgentMarkdown citations text={draft} sourceId={node.id} />
          )}
        </div>

        {outgoing.length > 0 || incoming.length > 0 ? (
          <section className="knowledge-document-structure" aria-label={i18n.t("knowledge.relatedNodes")}>
            <h2 className="knowledge-document-structure-title">{i18n.t("knowledge.relatedNodes")}</h2>
            {outgoing.length > 0 ? (
              <div className="knowledge-document-relation">
                <span className="knowledge-document-relation-label">{i18n.t("knowledge.refs")}</span>
                <div className="knowledge-document-relation-chips">
                  {outgoing.map((key) => (
                    <CitationChip
                      key={key}
                      citation={{ kind: "node", key }}
                      sourceId={node.id}
                    />
                  ))}
                </div>
              </div>
            ) : null}
            {incoming.length > 0 ? (
              <div className="knowledge-document-relation">
                <span className="knowledge-document-relation-label">{i18n.t("knowledge.referencedBy")}</span>
                <div className="knowledge-document-relation-chips">
                  {incoming.map((candidate) => (
                    <CitationChip
                      key={candidate.id}
                      citation={{ kind: "node", key: candidate.key }}
                    />
                  ))}
                </div>
              </div>
            ) : null}
          </section>
        ) : null}
      </article>
    </div>
  );
}
