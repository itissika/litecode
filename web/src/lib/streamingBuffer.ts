import { useEffect, useRef, useState } from "react";

const CHARACTER_DELAY_MS = 28;
const MIN_CHARACTERS_PER_TICK = 1;
const MAX_CHARACTERS_PER_TICK = 16;
const BACKLOG_FOR_MAX_SPEED = 1_000;

function nextCharacterEnd(text: string, start: number): number {
  const first = text.codePointAt(start);
  if (first === undefined) return start;

  let end = start + (first > 0xffff ? 2 : 1);
  // Keep combining marks and variation selectors attached to their base
  // character so a visible update never splits an accented glyph or emoji.
  while (end < text.length) {
    const next = text.codePointAt(end);
    if (next === undefined) break;
    const nextText = String.fromCodePoint(next);
    if (!/\p{Mark}/u.test(nextText) && next !== 0xfe0e && next !== 0xfe0f) {
      break;
    }
    end += next > 0xffff ? 2 : 1;
  }
  return end;
}

function advanceCharacters(text: string, start: number, count: number): number {
  let end = start;
  for (let i = 0; i < count && end < text.length; i += 1) {
    end = nextCharacterEnd(text, end);
  }
  return end;
}

function charactersPerTick(backlog: number): number {
  const clampedBacklog = Math.min(
    Math.max(backlog, 1),
    BACKLOG_FOR_MAX_SPEED,
  );
  const progress = (clampedBacklog - 1) / (BACKLOG_FOR_MAX_SPEED - 1);
  return Math.round(
    MIN_CHARACTERS_PER_TICK +
      progress * (MAX_CHARACTERS_PER_TICK - MIN_CHARACTERS_PER_TICK),
  );
}

/**
 * Smooths streaming text without making it a second source of truth.
 *
 * The source text is always complete and authoritative. This hook only exposes
 * a prefix for presentation, speeds up as its backlog grows, and immediately
 * catches up whenever the stream ends or authority replaces the text.
 */
export function useStreamingBuffer(text: string, streaming: boolean): string {
  const [displayText, setDisplayText] = useState(text);
  const sourceRef = useRef(text);
  const streamingRef = useRef(streaming);
  const displayRef = useRef(text);
  const timerRef = useRef<number | null>(null);

  sourceRef.current = text;
  streamingRef.current = streaming;

  useEffect(() => {
    const flush = (value: string) => {
      if (displayRef.current === value) return;
      displayRef.current = value;
      setDisplayText(value);
    };

    // A buffer/item seal can replace a partial stream value. It is not safe to
    // animate across a divergence, so immediately show the authoritative text.
    if (!text.startsWith(displayRef.current)) {
      flush(text);
      return;
    }

    if (!streaming) {
      flush(text);
      return;
    }

    const schedule = () => {
      if (timerRef.current !== null) return;
      timerRef.current = window.setTimeout(() => {
        timerRef.current = null;

        const source = sourceRef.current;
        const displayed = displayRef.current;
        if (!source.startsWith(displayed) || !streamingRef.current) {
          flush(source);
          return;
        }
        if (displayed.length >= source.length) return;

        const end = advanceCharacters(
          source,
          displayed.length,
          charactersPerTick(source.length - displayed.length),
        );
        flush(source.slice(0, end));
        schedule();
      }, CHARACTER_DELAY_MS);
    };

    if (displayRef.current.length < text.length) schedule();
  }, [text, streaming]);

  useEffect(
    () => () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
      }
    },
    [],
  );

  return displayText;
}
