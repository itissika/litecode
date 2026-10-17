import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";

import type { MarkdownBlock } from "../lib/markdownBlocks";
import { MarkdownBody, type MarkdownPreview } from "./MarkdownBody";

/** Mount blocks this far outside the visible scrollport before they enter. */
const VIEW_MARGIN = "240px 0px";

export const WindowedMarkdown = memo(function WindowedMarkdown({
  blocks,
  streaming,
  citations,
  sourceId,
  preview,
}: {
  blocks: MarkdownBlock[];
  streaming: boolean;
  citations: boolean;
  sourceId?: string;
  preview?: MarkdownPreview;
}) {
  const last = blocks.length - 1;
  return (
    <>
      {blocks.map((block, index) => (
        <MarkdownSlot
          key={index}
          text={block.text}
          estimatedHeight={block.estimatedHeight}
          streaming={streaming && index === last}
          citations={citations}
          sourceId={sourceId}
          preview={preview}
          gap={index !== last}
        />
      ))}
    </>
  );
});

const MarkdownSlot = memo(function MarkdownSlot({
  text,
  estimatedHeight,
  streaming,
  citations,
  sourceId,
  preview,
  gap,
}: {
  text: string;
  estimatedHeight: number;
  streaming: boolean;
  citations: boolean;
  sourceId?: string;
  preview?: MarkdownPreview;
  gap: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const measured = useRef<number | null>(null);
  const measuredText = useRef<string | null>(null);
  const [shown, setShown] = useState(
    () => typeof IntersectionObserver === "undefined",
  );

  useLayoutEffect(() => {
    const el = ref.current;
    if (!shown || !el) return;
    const measure = () => {
      const next = el.offsetHeight;
      if (next <= 0) return;
      measured.current = next;
      measuredText.current = text;
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [shown, text]);

  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry) return;
        setShown((prev) =>
          prev === entry.isIntersecting ? prev : entry.isIntersecting,
        );
      },
      { rootMargin: VIEW_MARGIN },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const height =
    measured.current != null && measuredText.current === text
      ? measured.current
      : estimatedHeight;

  return (
    <div
      ref={ref}
      className={gap ? "mb-2.5" : undefined}
      style={shown ? undefined : { height }}
    >
      {shown ? (
        <MarkdownBody
          text={text}
          streaming={streaming}
          citations={citations}
          sourceId={sourceId}
          preview={preview}
        />
      ) : null}
    </div>
  );
});
