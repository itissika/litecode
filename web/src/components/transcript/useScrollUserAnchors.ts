import { useCallback, useEffect, useRef } from "react";

import type { UserAnchorsResult } from "../../api/types";
import { useConnectionStore } from "../../stores/connectionStore";
import { USER_RAIL_RADIUS } from "./transcriptUserRailMarks";

/**
 * User-anchor window for the rail. A request runs when scrolling moves the
 * viewport midline onto a different user message, not when the panel mounts.
 */
export function useScrollUserAnchors(sessionId: string) {
  const connState = useConnectionStore((s) => s.state);
  const lastSeq = useRef<number | null>(null);
  const pending = useRef<number | null>(null);
  const gen = useRef(0);
  const connRef = useRef(connState);
  connRef.current = connState;

  useEffect(() => {
    lastSeq.current = null;
    pending.current = null;
    gen.current += 1;
  }, [sessionId]);

  const noteCenter = useCallback(
    (seq: number) => {
      pending.current = seq;
      if (connRef.current !== "connected") return;
      if (lastSeq.current === seq) return;
      lastSeq.current = seq;
      const ticket = ++gen.current;
      void useConnectionStore
        .getState()
        .sendRpc<UserAnchorsResult>("buffer/user-anchors", {
          session_id: sessionId,
          anchor_seq: seq,
          before: USER_RAIL_RADIUS,
          after: USER_RAIL_RADIUS,
        })
        .then(() => {
          if (ticket !== gen.current) return;
        })
        .catch(() => {
          if (ticket !== gen.current) return;
          lastSeq.current = null;
        });
    },
    [sessionId],
  );

  useEffect(() => {
    if (connState !== "connected") return;
    const seq = pending.current;
    if (seq == null) return;
    noteCenter(seq);
  }, [connState, noteCenter]);

  return { noteCenter };
}
