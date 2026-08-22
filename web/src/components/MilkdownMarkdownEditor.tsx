import { Crepe } from "@milkdown/crepe";
import { replaceAll } from "@milkdown/kit/utils";
import { useEffect, useRef } from "react";

import { registerMarkdownFlush } from "../lib/markdownFlush";

import "@milkdown/crepe/theme/common/style.css";

type Props = {
  filePath: string;
  content: string;
  onChange: (markdown: string) => void;
};

export function MilkdownMarkdownEditor({ filePath, content, onChange }: Props) {
  const rootRef = useRef<HTMLDivElement>(null);
  const crepeRef = useRef<Crepe | null>(null);
  const onChangeRef = useRef(onChange);
  const lastEmittedRef = useRef(content);
  const applyingRef = useRef(false);
  const acceptUpdatesRef = useRef(false);
  const userEditedRef = useRef(false);

  onChangeRef.current = onChange;

  const contentRef = useRef(content);
  contentRef.current = content;

  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;

    const initial = contentRef.current;
    acceptUpdatesRef.current = false;
    userEditedRef.current = false;
    applyingRef.current = false;
    lastEmittedRef.current = initial;
    root.replaceChildren();

    const crepe = new Crepe({
      root,
      defaultValue: initial,
      features: {
        [Crepe.Feature.AI]: false,
        [Crepe.Feature.TopBar]: false,
      },
    });
    crepeRef.current = crepe;

    crepe.on((listener) => {
      listener.markdownUpdated((_ctx, markdown) => {
        if (applyingRef.current || !acceptUpdatesRef.current) return;
        if (markdown === lastEmittedRef.current) return;
        userEditedRef.current = true;
        lastEmittedRef.current = markdown;
        onChangeRef.current(markdown);
      });
    });

    let cancelled = false;
    void crepe.create().then(() => {
      if (cancelled) {
        void crepe.destroy();
        return;
      }
      const latest = contentRef.current;
      if (latest !== initial) {
        applyingRef.current = true;
        lastEmittedRef.current = latest;
        userEditedRef.current = false;
        try {
          crepe.editor.action(replaceAll(latest, true));
        } catch {
          // Parser rejected the buffer; keep defaultValue.
        }
        applyingRef.current = false;
      }
      requestAnimationFrame(() => {
        if (!cancelled) acceptUpdatesRef.current = true;
      });
    });

    const unregister = registerMarkdownFlush(filePath, () => {
      if (!userEditedRef.current || !crepeRef.current) return null;
      const markdown = crepeRef.current.getMarkdown();
      lastEmittedRef.current = markdown;
      return markdown;
    });

    return () => {
      cancelled = true;
      acceptUpdatesRef.current = false;
      unregister();
      crepeRef.current = null;
      void crepe.destroy();
    };
    // Mount once per file tab. Disk/agent updates go through replaceAll below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filePath]);

  useEffect(() => {
    const crepe = crepeRef.current;
    if (!crepe || content === lastEmittedRef.current) return;
    applyingRef.current = true;
    lastEmittedRef.current = content;
    userEditedRef.current = false;
    try {
      crepe.editor.action(replaceAll(content, true));
    } catch {
      // Editor not created yet; defaultValue already matches the open buffer.
    }
    applyingRef.current = false;
  }, [content]);

  return <div ref={rootRef} className="milkdown-host h-full" />;
}
