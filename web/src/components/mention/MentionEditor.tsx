import Document from "@tiptap/extension-document";
import Mention from "@tiptap/extension-mention";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import { Placeholder } from "@tiptap/extensions/placeholder";
import { UndoRedo } from "@tiptap/extensions/undo-redo";
import type { Editor, Range } from "@tiptap/core";
import { splitBlock } from "@tiptap/pm/commands";
import type { Slice } from "@tiptap/pm/model";
import { PluginKey } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";
import {
  EditorContent,
  ReactNodeViewRenderer,
  useEditor,
} from "@tiptap/react";
import { exitSuggestion, type SuggestionProps } from "@tiptap/suggestion";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import {
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type Ref,
} from "react";

import { trackEditorView } from "../../lib/editorRoot";

import { citationFact, formatLineSpan } from "../../lib/knowledge/markers";
import { dragCarriesMention, dropCarriesMention } from "../../lib/dropPayload";
import { mentionTextForDrop } from "../../lib/mentionDrop";
import { useSessionStore } from "../../stores/sessionStore";
import { MentionChipView } from "./chips";
import { bodyToContent, fileMentionOptions, knowledgeMentionOptions, mentionInlineContent } from "./serialize";
import {
  fileCandidates,
  fileDisplayName,
  filterSymbols,
  lockedQuery,
  mentionItems,
  phaseForQuery,
  symbolCandidate,
  symbolsFor,
  type FileCandidate,
  type LockedFile,
  type SymbolCandidate,
} from "./suggestions";

export interface MentionEditorHandle {
  focus: () => void;
  insertText: (text: string) => void;
}

type MenuItem =
  | { kind: "node"; id: string }
  | ({ kind: "file" } & FileCandidate)
  | ({ kind: "symbol" } & SymbolCandidate);

type MentionAttrs = {
  id: string;
  label: string;
  symbol?: string | null;
  lines?: string | null;
};

type MentionListHandle = {
  onKeyDown: (props: { event: KeyboardEvent }) => boolean;
};

type MentionListProps = SuggestionProps<MenuItem, MentionAttrs> & {
  wide?: boolean;
  includeLinesRef?: { current: boolean };
  onHighlight?: (path: string | null) => void;
  onLock?: (path: string) => void;
};

type MentionMenu = {
  update: (next: MentionListProps) => void;
  onKeyDown: (props: { event: KeyboardEvent }) => boolean;
  destroy: () => void;
};

/**
 * The suggestion plugin mounts into the opener `document.body`. A popped-out
 * panel lives in another document, so the menu has to be created there or it
 * paints on the main window.
 */
function mountMentionList(props: MentionListProps, pluginKey: PluginKey): MentionMenu {
  const doc = props.editor.view.dom.ownerDocument;
  const host = doc.createElement("div");
  host.className = "knowledge-mention-menu";
  doc.body.appendChild(host);
  const root = createRoot(host);
  const handle: { current: MentionListHandle | null } = { current: null };
  let closed = false;

  const paint = (next: MentionListProps) => {
    flushSync(() => {
      root.render(
        <MentionList
          {...next}
          ref={(value) => {
            handle.current = value;
          }}
        />,
      );
    });
  };

  const releasePosition = props.mount(host, {
    autoUpdate: { animationFrame: true },
  });

  let stopOutside = () => {};
  if (doc !== document) {
    const NodeCtor = doc.defaultView?.Node ?? Node;
    const onPointerDown = (event: Event) => {
      const target = event.target;
      if (!target || !(target instanceof NodeCtor)) return;
      if (host.contains(target) || props.editor.view.dom.contains(target)) return;
      exitSuggestion(props.editor.view, pluginKey);
    };
    doc.addEventListener("pointerdown", onPointerDown, true);
    stopOutside = () => doc.removeEventListener("pointerdown", onPointerDown, true);
  }

  paint(props);

  return {
    update(next) {
      if (!closed) paint(next);
    },
    onKeyDown(event) {
      return handle.current?.onKeyDown(event) ?? false;
    },
    destroy() {
      if (closed) return;
      closed = true;
      stopOutside();
      releasePosition();
      root.unmount();
      host.remove();
    },
  };
}

interface MentionEditorProps {
  label: string;
  sourceId?: string;
  value: string;
  candidates: readonly string[];
  rows?: number;
  placeholder?: string;
  className?: string;
  style?: CSSProperties;
  symbolLines?: boolean;
  submitOnEnter?: boolean;
  onChange: (next: string) => void;
  onBlur?: () => void;
  onSubmit?: () => void;
  onFocus?: () => void;
  onPaste?: (event: ClipboardEvent) => boolean;
  onDropText?: (text: string) => boolean;
  /** File, path, and code-span drops become mention chips at the caret. */
  onMentionDrop?: boolean;
  onEscape?: () => void;
  handle?: Ref<MentionEditorHandle>;
}

function editorText(editor: { getText: (options: { blockSeparator: string }) => string }) {
  return editor.getText({ blockSeparator: "\n" });
}

function MentionList({
  ref,
  items,
  command,
  loading,
  wide,
  includeLinesRef,
  onHighlight,
  onLock,
}: SuggestionProps<MenuItem, MentionAttrs> & {
  ref?: Ref<MentionListHandle>;
  wide?: boolean;
  includeLinesRef?: { current: boolean };
  onHighlight?: (path: string | null) => void;
  onLock?: (path: string) => void;
}) {
  const [selectedIndex, setSelectedIndex] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);

  const onHighlightRef = useRef(onHighlight);
  onHighlightRef.current = onHighlight;

  function highlightIndex(index: number) {
    const item = items[index];
    onHighlightRef.current?.(item?.kind === "file" && item.file ? item.path : null);
  }

  function selectItem(index: number) {
    const item = items[index];
    if (!item) return;
    if (item.kind === "node") {
      command({ id: item.id, label: item.id });
      return;
    }
    if (item.kind === "file") {
      command({ id: item.path, label: item.path });
      return;
    }
    const lines =
      includeLinesRef?.current && item.start > 0
        ? formatLineSpan(item.start, item.end)
        : null;
    command({
      id: item.path,
      label: citationFact(item.path, item.chain, lines),
      symbol: item.chain,
      lines,
    });
  }

  useEffect(() => {
    setSelectedIndex(0);
  }, [items]);

  useEffect(() => {
    rootRef.current?.parentElement?.classList.toggle("is-symbols", wide === true);
  }, [wide]);

  useEffect(() => {
    highlightIndex(selectedIndex);
    const menu = rootRef.current?.parentElement;
    const active = rootRef.current?.querySelector<HTMLElement>(".is-active");
    if (!menu || !active) return;
    const menuRect = menu.getBoundingClientRect();
    const itemRect = active.getBoundingClientRect();
    if (itemRect.top < menuRect.top) {
      menu.scrollTop -= menuRect.top - itemRect.top;
    } else if (itemRect.bottom > menuRect.bottom) {
      menu.scrollTop += itemRect.bottom - menuRect.bottom;
    }
  }, [items, selectedIndex]);

  useImperativeHandle(ref, () => ({
    onKeyDown: ({ event }) => {
      if (items.length === 0) return false;
      if (event.key === "ArrowUp") {
        const next = (selectedIndex + items.length - 1) % items.length;
        setSelectedIndex(next);
        highlightIndex(next);
        return true;
      }
      if (event.key === "ArrowDown") {
        const next = (selectedIndex + 1) % items.length;
        setSelectedIndex(next);
        highlightIndex(next);
        return true;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        selectItem(selectedIndex);
        return true;
      }
      return false;
    },
  }));

  if (items.length === 0) {
    return (
      <div ref={rootRef} className="knowledge-mention-empty">
        {loading ? "Searching…" : "No matches"}
      </div>
    );
  }

  return (
    <div ref={rootRef} className="knowledge-mention-list" role="listbox">
      {items.map((item, index) => {
        const active = index === selectedIndex;
        const className = active ? "knowledge-complete-item is-active" : "knowledge-complete-item";
        if (item.kind === "symbol") {
          return (
            <button
              type="button"
              role="option"
              aria-selected={active}
              className={`${className} knowledge-symbol-row`}
              key={`${item.path}:${item.chain}:${item.start}`}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => selectItem(index)}
            >
              <span className="knowledge-symbol-chain">
                <span>{item.chain}</span>
              </span>
              {item.summary ? <span className="knowledge-symbol-summary">{item.summary}</span> : null}
            </button>
          );
        }
        if (item.kind === "file") {
          return (
            <div
              role="option"
              aria-selected={active}
              className={`${className} knowledge-mention-file`}
              key={item.path}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => selectItem(index)}
            >
              <span className="knowledge-mention-path">{item.path}</span>
              {item.file ? (
                <button
                  type="button"
                  className="knowledge-mention-hash"
                  aria-label={`Pick a symbol in ${fileDisplayName(item.path)}`}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={(event) => {
                    event.stopPropagation();
                    onLock?.(item.path);
                  }}
                >
                  #
                </button>
              ) : null}
            </div>
          );
        }
        return (
          <button
            type="button"
            role="option"
            aria-selected={active}
            className={className}
            key={item.id}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => selectItem(index)}
          >
            {item.id}
          </button>
        );
      })}
    </div>
  );
}

function nodeSuggestion(candidatesRef: { current: readonly string[] }, pluginKey: PluginKey) {
  return {
    char: "@",
    pluginKey,
    items: ({ query }: { query: string }) =>
      mentionItems(candidatesRef.current, query).map((id) => ({ kind: "node" as const, id })),
    render: () => {
      let menu: MentionMenu | null = null;
      return {
        onStart: (props: SuggestionProps<MenuItem, MentionAttrs>) => {
          menu = mountMentionList(props, pluginKey);
        },
        onUpdate(props: SuggestionProps<MenuItem, MentionAttrs>) {
          menu?.update(props);
        },
        onKeyDown(props: { event: KeyboardEvent }) {
          return menu?.onKeyDown(props) ?? false;
        },
        onExit() {
          menu?.destroy();
          menu = null;
        },
      };
    },
  };
}

function fileSuggestion(
  pluginKey: PluginKey,
  includeLinesRef: { current: boolean },
) {
  const locked: { current: LockedFile | null } = { current: null };
  const highlighted: { current: string | null } = { current: null };
  return {
    char: "/",
    pluginKey,
    items: async ({ query, signal }: { query: string; signal?: AbortSignal }) => {
      const phase = phaseForQuery(query, locked.current);
      if (phase.mode === "file") {
        const paths = await fileCandidates(phase.filter, signal);
        return paths.map((item) => ({ kind: "file" as const, ...item }));
      }
      try {
        const symbols = filterSymbols(await symbolsFor(phase.locked.path), phase.filter);
        return symbols.map((item) => ({
          kind: "symbol" as const,
          ...symbolCandidate(phase.locked.path, item),
        }));
      } catch {
        return [];
      }
    },
    render: () => {
      let menu: MentionMenu | null = null;
      let editor: Editor | null = null;
      let range: Range = { from: 0, to: 0 };

      function lock(path: string) {
        if (!editor) return;
        const name = fileDisplayName(path);
        locked.current = { path, name };
        editor.chain().focus().insertContentAt(range, lockedQuery(name)).run();
      }

      function listProps(props: SuggestionProps<MenuItem, MentionAttrs>) {
        const wide = phaseForQuery(props.query, locked.current).mode === "symbol";
        return {
          ...props,
          wide,
          includeLinesRef,
          onHighlight: (path: string | null) => {
            highlighted.current = path;
          },
          onLock: lock,
        };
      }

      return {
        onStart: (props: SuggestionProps<MenuItem, MentionAttrs>) => {
          editor = props.editor;
          range = props.range;
          menu = mountMentionList(listProps(props), pluginKey);
        },
        onUpdate(props: SuggestionProps<MenuItem, MentionAttrs>) {
          editor = props.editor;
          range = props.range;
          if (phaseForQuery(props.query, locked.current).mode === "file") locked.current = null;
          menu?.update(listProps(props));
        },
        onKeyDown(props: { event: KeyboardEvent; range: Range }) {
          range = props.range;
          if (props.event.key === "#" && !locked.current && highlighted.current) {
            props.event.preventDefault();
            lock(highlighted.current);
            return true;
          }
          return menu?.onKeyDown(props) ?? false;
        },
        onExit() {
          locked.current = null;
          highlighted.current = null;
          menu?.destroy();
          menu = null;
        },
      };
    },
  };
}

export function MentionEditor({
  label,
  sourceId = "",
  value,
  candidates,
  rows,
  placeholder = "",
  className,
  style,
  symbolLines = false,
  submitOnEnter = false,
  onChange,
  onBlur,
  onSubmit,
  onFocus,
  onPaste,
  onDropText,
  onMentionDrop,
  onEscape,
  handle,
}: MentionEditorProps) {
  const sourceIdRef = useRef(sourceId);
  sourceIdRef.current = sourceId;
  const candidatesRef = useRef(candidates);
  candidatesRef.current = candidates;
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const onBlurRef = useRef(onBlur);
  onBlurRef.current = onBlur;
  const onSubmitRef = useRef(onSubmit);
  onSubmitRef.current = onSubmit;
  const onFocusRef = useRef(onFocus);
  onFocusRef.current = onFocus;
  const onPasteRef = useRef(onPaste);
  onPasteRef.current = onPaste;
  const onDropTextRef = useRef(onDropText);
  onDropTextRef.current = onDropText;
  const onMentionDropRef = useRef(onMentionDrop);
  onMentionDropRef.current = onMentionDrop;
  const onEscapeRef = useRef(onEscape);
  onEscapeRef.current = onEscape;
  const placeholderRef = useRef(placeholder);
  placeholderRef.current = placeholder;
  const includeLinesRef = useRef(symbolLines);
  includeLinesRef.current = symbolLines;
  const submitRef = useRef(submitOnEnter);
  submitRef.current = submitOnEnter;
  const emitted = useRef(value);
  const initialContent = useRef(bodyToContent(value));
  const insertChips = useRef<((pos: number, text: string) => void) | null>(null);
  const nodeKey = useMemo(() => new PluginKey("knowledgeNodeMention"), []);
  const fileKey = useMemo(() => new PluginKey("knowledgeFileMention"), []);

  const extensions = useMemo(
    () => [
      Document,
      Paragraph,
      Text,
      UndoRedo,
      Placeholder.configure({
        placeholder: () => placeholderRef.current,
      }),
      Mention.extend({
        addNodeView() {
          return ReactNodeViewRenderer(
            (props) => (
              <MentionChipView {...props} sourceIdRef={sourceIdRef} mode="node" />
            ),
            { as: "span" },
          );
        },
      }).configure(knowledgeMentionOptions(nodeSuggestion(candidatesRef, nodeKey))),
      Mention.extend({
        name: "fileMention",
        addAttributes() {
          return {
            ...this.parent?.(),
            symbol: { default: null },
            lines: { default: null },
          };
        },
        addNodeView() {
          return ReactNodeViewRenderer(
            (props) => (
              <MentionChipView {...props} sourceIdRef={sourceIdRef} mode="file" />
            ),
            { as: "span" },
          );
        },
      }).configure(fileMentionOptions(fileSuggestion(fileKey, includeLinesRef))),
    ],
    [fileKey, nodeKey],
  );

  const editorProps = useMemo(
    () => ({
      attributes: {
        class: "mention-editor-body",
        spellcheck: "false",
        "aria-label": label,
      },
      handleKeyDown: (view: EditorView, event: KeyboardEvent) => {
        const suggesting =
          nodeKey.getState(view.state)?.active || fileKey.getState(view.state)?.active;
        if (event.key === "Escape" && onEscapeRef.current && !suggesting) {
          event.preventDefault();
          onEscapeRef.current();
          return true;
        }
        // Shift+Enter is the "soft" newline. Nothing else binds the chord —
        // the base keymap has no Shift-Enter and this schema has no hardBreak
        // node, so the browser's own <br> would be parsed away. One line is
        // one paragraph, which is exactly how the body text round-trips.
        if (event.key === "Enter" && event.shiftKey) {
          event.preventDefault();
          return splitBlock(view.state, view.dispatch);
        }
        if (!submitRef.current || event.key !== "Enter") return false;
        if (suggesting) return false;
        event.preventDefault();
        onSubmitRef.current?.();
        return true;
      },
      handlePaste: (_view: EditorView, event: ClipboardEvent) => onPasteRef.current?.(event) === true,
      handleDrop: (view: EditorView, event: DragEvent, _slice: Slice, moved: boolean) => {
        if (moved) return false;
        const transfer = event.dataTransfer;
        if (onMentionDropRef.current && transfer && dropCarriesMention(transfer)) {
          event.preventDefault();
          event.stopPropagation();
          const coords = view.posAtCoords({ left: event.clientX, top: event.clientY });
          const pos = coords?.pos ?? view.state.selection.from;
          const project = useSessionStore.getState().project;
          void mentionTextForDrop(transfer, project).then((text) => {
            if (text) insertChips.current?.(pos, text);
          });
          return true;
        }
        const text = transfer?.getData("text/plain") ?? "";
        if (!text || onDropTextRef.current?.(text) !== true) return false;
        event.preventDefault();
        return true;
      },
      handleDOMEvents: {
        dragover: (_view: EditorView, event: DragEvent) => {
          const transfer = event.dataTransfer;
          if (!onMentionDropRef.current || !transfer || !dragCarriesMention(transfer)) {
            return false;
          }
          event.preventDefault();
          return true;
        },
        blur: () => {
          onBlurRef.current?.();
          return false;
        },
        focus: () => {
          onFocusRef.current?.();
          return false;
        },
      },
    }),
    [fileKey, label, nodeKey],
  );

  const editor = useEditor({
    immediatelyRender: true,
    extensions,
    content: initialContent.current,
    editorProps,
    onUpdate: ({ editor: current }) => {
      const next = editorText(current);
      if (next === emitted.current) return;
      emitted.current = next;
      onChangeRef.current(next);
    },
  });

  useLayoutEffect(() => {
    if (!editor) return;
    return trackEditorView(editor.view, () => {
      exitSuggestion(editor.view, nodeKey);
      exitSuggestion(editor.view, fileKey);
    });
  }, [editor, fileKey, nodeKey]);

  insertChips.current = (pos, text) => {
    if (!editor || editor.isDestroyed) return;
    const nodes = mentionInlineContent(text);
    if (nodes.length === 0) return;
    editor.chain().focus().insertContentAt(pos, nodes).run();
  };

  useImperativeHandle(handle, () => ({
    focus: () => {
      editor?.commands.focus();
    },
    insertText: (text: string) => {
      if (!editor) return;
      editor.chain().focus("end").insertContent(text).run();
      const next = editorText(editor);
      emitted.current = next;
      onChangeRef.current(next);
    },
  }), [editor]);

  useEffect(() => {
    if (!editor) return;
    (editor.view.dom as HTMLElement & { litecodeEditor?: typeof editor }).litecodeEditor = editor;
  }, [editor]);

  useEffect(() => {
    if (!editor) return;
    const dom = editor.view.dom;
    if (onMentionDrop) dom.setAttribute("data-mention-drop", "true");
    else dom.removeAttribute("data-mention-drop");
  }, [editor, onMentionDrop]);

  useEffect(() => {
    if (!editor || editor.isDestroyed) return;
    // Decorations read `placeholderRef`. An empty transaction refreshes
    // `data-placeholder` when the hint changes without an edit.
    editor.view.dispatch(editor.state.tr);
  }, [editor, placeholder]);

  useEffect(() => {
    if (!editor) return;
    if (value === emitted.current) return;
    if (value === editorText(editor)) {
      emitted.current = value;
      return;
    }
    editor.commands.setContent(bodyToContent(value), { emitUpdate: false });
    emitted.current = value;
  }, [editor, value]);

  const minHeight =
    rows != null ? { minHeight: `${Math.max(rows, 3) * 1.15}rem` } : undefined;

  return (
    <EditorContent
      editor={editor}
      className={className}
      style={style ?? minHeight}
    />
  );
}
