import { X } from "@phosphor-icons/react";
import Document from "@tiptap/extension-document";
import Mention from "@tiptap/extension-mention";
import Paragraph from "@tiptap/extension-paragraph";
import Text from "@tiptap/extension-text";
import { PluginKey } from "@tiptap/pm/state";
import {
  EditorContent,
  NodeViewWrapper,
  ReactNodeViewRenderer,
  ReactRenderer,
  useEditor,
  type ReactNodeViewProps,
} from "@tiptap/react";
import type { SuggestionProps } from "@tiptap/suggestion";
import {
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type Ref,
} from "react";

import { fetchGlob } from "../../api/workspace";
import { fileLabel } from "../../lib/knowledge/markers";
import {
  bodyToContent,
  fileMentionOptions,
  knowledgeMentionOptions,
} from "../../lib/knowledge/mentionDoc";
import { chipForMarker } from "../../lib/knowledge/refDisplay";
import { useEditorStore } from "../../stores/editorStore";
import { useKnowledgeStore } from "../../stores/knowledgeStore";

type MentionAttrs = { id: string; label: string };

type MentionListHandle = {
  onKeyDown: (props: { event: KeyboardEvent }) => boolean;
};

function mentionItems(candidates: readonly string[], query: string, limit = 8): string[] {
  const q = query.trim();
  const matched = q ? candidates.filter((key) => key.startsWith(q)) : candidates;
  return matched.slice(0, limit);
}

/**
 * Suggestion popup from the TipTap mention example:
 * ReactRenderer plus `props.mount`, which positions the list with Floating UI.
 */
const nodeMentionKey = new PluginKey("knowledgeNodeMention");
const fileMentionKey = new PluginKey("knowledgeFileMention");

function mentionSuggestion(candidatesRef: { current: readonly string[] }) {
  return {
    char: "@",
    pluginKey: nodeMentionKey,
    items: ({ query }: { query: string }) => mentionItems(candidatesRef.current, query),
    render: () => {
      let component: ReactRenderer<MentionListHandle> | null = null;
      let unmount: (() => void) | null = null;
      return {
        onStart: (props: SuggestionProps<string, MentionAttrs>) => {
          component = new ReactRenderer(MentionList, {
            props,
            editor: props.editor,
            className: "knowledge-mention-menu",
          });
          unmount = props.mount(component.element, {
            autoUpdate: { animationFrame: true },
          });
        },
        onUpdate(props: SuggestionProps<string, MentionAttrs>) {
          component?.updateProps(props);
        },
        onKeyDown(props: { event: KeyboardEvent }) {
          return component?.ref?.onKeyDown(props) ?? false;
        },
        onExit() {
          unmount?.();
          unmount = null;
          component?.destroy();
          component = null;
        },
      };
    },
  };
}

function MentionList({
  ref,
  items,
  command,
  labelFor,
  loading,
}: SuggestionProps<string, MentionAttrs> & {
  ref?: Ref<MentionListHandle>;
  labelFor?: (item: string) => string;
}) {
  const [selectedIndex, setSelectedIndex] = useState(0);

  function selectItem(index: number) {
    const item = items[index];
    if (!item) return;
    command({ id: item, label: labelFor ? labelFor(item) : item });
  }

  useEffect(() => {
    setSelectedIndex(0);
  }, [items]);

  useImperativeHandle(ref, () => ({
    onKeyDown: ({ event }) => {
      if (items.length === 0) return false;
      if (event.key === "ArrowUp") {
        setSelectedIndex((index) => (index + items.length - 1) % items.length);
        return true;
      }
      if (event.key === "ArrowDown") {
        setSelectedIndex((index) => (index + 1) % items.length);
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
      <div className="knowledge-mention-empty">{loading ? "搜索中" : "未找到"}</div>
    );
  }

  return (
    <div className="knowledge-mention-list" role="listbox">
      {items.map((item, index) => (
        <button
          type="button"
          role="option"
          aria-selected={index === selectedIndex}
          className={
            index === selectedIndex
              ? "knowledge-complete-item is-active"
              : "knowledge-complete-item"
          }
          key={item}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => selectItem(index)}
        >
          {item}
        </button>
      ))}
    </div>
  );
}

async function fileCandidates(query: string): Promise<string[]> {
  const cleaned = query
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+/, "")
    .replace(/[*?[\]]/g, "");
  if (!cleaned || cleaned.includes("..")) return [];
  try {
    const listing = await fetchGlob(`**/${cleaned}*`);
    const paths: string[] = [];
    for (const entry of listing.entries) {
      if (entry.kind !== "file" && entry.kind !== "dir") continue;
      paths.push(entry.path.replaceAll("\\", "/"));
      if (paths.length === 8) break;
    }
    return paths;
  } catch {
    return [];
  }
}

function fileMentionSuggestion() {
  return {
    char: "/",
    pluginKey: fileMentionKey,
    items: ({ query }: { query: string }) => fileCandidates(query),
    render: () => {
      let component: ReactRenderer<MentionListHandle> | null = null;
      let unmount: (() => void) | null = null;
      const labelFor = (path: string) => fileLabel(path);
      return {
        onStart: (props: SuggestionProps<string, MentionAttrs>) => {
          component = new ReactRenderer(MentionList, {
            props: { ...props, labelFor },
            editor: props.editor,
            className: "knowledge-mention-menu",
          });
          unmount = props.mount(component.element, {
            autoUpdate: { animationFrame: true },
          });
        },
        onUpdate(props: SuggestionProps<string, MentionAttrs>) {
          component?.updateProps({ ...props, labelFor });
        },
        onKeyDown(props: { event: KeyboardEvent }) {
          return component?.ref?.onKeyDown(props) ?? false;
        },
        onExit() {
          unmount?.();
          unmount = null;
          component?.destroy();
          component = null;
        },
      };
    },
  };
}

function stopChipEvent(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
}

/** Editor chip. The label jumps; only the remove control deletes the mention. */
function KnowledgeMentionChip({
  node,
  deleteNode,
  sourceIdRef,
}: ReactNodeViewProps & { sourceIdRef: { current: string } }) {
  const id = String(node.attrs.id ?? "");
  const label = String(node.attrs.label ?? id);
  const source = useKnowledgeStore((s) => s.byId.get(sourceIdRef.current));
  const byKey = useKnowledgeStore((s) => s.byKey);
  const focusCanvas = useKnowledgeStore((s) => s.focusCanvas);
  const model = source ? chipForMarker(source, id, byKey, label) : null;
  const jumpable = model?.jumpable === true && model.targetId != null;

  return (
    <NodeViewWrapper
      as="span"
      className={
        model?.tone === "error" ? "knowledge-token is-invalid" : "knowledge-token"
      }
    >
      {jumpable ? (
        <button
          type="button"
          className="knowledge-token-label nodrag"
          aria-label={label}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            if (model?.targetId) focusCanvas(model.targetId);
          }}
        >
          {label}
        </button>
      ) : (
        <span className="knowledge-token-label">{label}</span>
      )}
      <button
        type="button"
        className="knowledge-token-remove nodrag"
        aria-label={`移除 ${label}`}
        onMouseDown={stopChipEvent}
        onClick={(event) => {
          stopChipEvent(event);
          deleteNode();
        }}
      >
        <X size={10} weight="bold" />
      </button>
    </NodeViewWrapper>
  );
}

/** Workspace path capsule. Blue until a check marks the path missing, then red. */
function FileMentionChip({
  node,
  deleteNode,
  sourceIdRef,
}: ReactNodeViewProps & { sourceIdRef: { current: string } }) {
  const path = String(node.attrs.id ?? "");
  const label = String(node.attrs.label ?? fileLabel(path));
  const missing = useKnowledgeStore((state) =>
    (state.issuesByNode.get(sourceIdRef.current) ?? []).some(
      (issue) => issue.code === "missing_file" && issue.ref === path,
    ),
  );

  return (
    <NodeViewWrapper
      as="span"
      className={
        missing ? "knowledge-token is-file is-missing" : "knowledge-token is-file"
      }
    >
      {missing ? (
        <span className="knowledge-token-label">{label}</span>
      ) : (
        <button
          type="button"
          className="knowledge-token-label nodrag"
          aria-label={label}
          onMouseDown={stopChipEvent}
          onClick={(event) => {
            stopChipEvent(event);
            void useEditorStore.getState().openFile(path);
          }}
        >
          {label}
        </button>
      )}
      <button
        type="button"
        className="knowledge-token-remove nodrag"
        aria-label={`移除 ${label}`}
        onMouseDown={stopChipEvent}
        onClick={(event) => {
          stopChipEvent(event);
          deleteNode();
        }}
      >
        <X size={10} weight="bold" />
      </button>
    </NodeViewWrapper>
  );
}

function editorText(editor: { getText: (options: { blockSeparator: string }) => string }) {
  return editor.getText({ blockSeparator: "\n" });
}

export function KnowledgeBodyEditor({
  label,
  sourceId,
  value,
  candidates,
  rows,
  onChange,
  onBlur,
}: {
  label: string;
  sourceId: string;
  value: string;
  candidates: readonly string[];
  rows: number;
  onChange: (next: string) => void;
  onBlur?: () => void;
}) {
  const sourceIdRef = useRef(sourceId);
  sourceIdRef.current = sourceId;
  const candidatesRef = useRef(candidates);
  candidatesRef.current = candidates;
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;
  const onBlurRef = useRef(onBlur);
  onBlurRef.current = onBlur;
  const emitted = useRef(value);
  const initialContent = useRef(bodyToContent(value));
  const extensions = useMemo(
    () => [
      Document,
      Paragraph,
      Text,
      Mention.extend({
        addNodeView() {
          return ReactNodeViewRenderer(
            (props) => <KnowledgeMentionChip {...props} sourceIdRef={sourceIdRef} />,
            { as: "span" },
          );
        },
      }).configure(knowledgeMentionOptions(mentionSuggestion(candidatesRef))),
      Mention.extend({
        name: "fileMention",
        addNodeView() {
          return ReactNodeViewRenderer(
            (props) => <FileMentionChip {...props} sourceIdRef={sourceIdRef} />,
            { as: "span" },
          );
        },
      }).configure(fileMentionOptions(fileMentionSuggestion())),
    ],
    [],
  );
  const editorProps = useMemo(
    () => ({
      attributes: {
        class: "knowledge-source-body",
        spellcheck: "false",
        "aria-label": label,
      },
      handleDOMEvents: {
        blur: () => {
          onBlurRef.current?.();
          return false;
        },
      },
    }),
    [label],
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

  return (
    <EditorContent
      editor={editor}
      className="knowledge-source-input knowledge-source-body"
      style={{ minHeight: `${Math.max(rows, 3) * 1.15}rem` }}
    />
  );
}
