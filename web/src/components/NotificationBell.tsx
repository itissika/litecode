import { useEffect, useRef } from "react";
import { Bell, Trash } from "@phosphor-icons/react";
import { motion, useReducedMotion } from "motion/react";
import {
  sessionNotificationItems,
  sessionNotificationLastSeen,
  useNotificationStore,
} from "../stores/notificationStore";
import { Popover } from "./ui/Popover";
import { ShapeBlur } from "./ShapeBlur";

const KIND_BORDER: Record<string, string> = {
  error: "border-(--_dk-red-500)",
  success: "border-(--_dk-emerald-500)",
  info: "border-(--_dk-line-visible)",
};

const KIND_DOT: Record<string, string> = {
  error: "bg-(--_dk-red-500)",
  success: "bg-(--_dk-emerald-500)",
  info: "bg-(--_dk-text-muted)",
};

export function NotificationBell({ sessionId }: { sessionId: string }) {
  const items = useNotificationStore((s) =>
    sessionNotificationItems(s.bySession, sessionId),
  );
  const lastSeen = useNotificationStore((s) =>
    sessionNotificationLastSeen(s.bySession, sessionId),
  );
  const clear = useNotificationStore((s) => s.clear);
  const markSeen = useNotificationStore((s) => s.markSeen);
  const prevCount = useRef(items.length);
  const reduceMotion = useReducedMotion();

  const hasNew = items.length > lastSeen;

  useEffect(() => {
    prevCount.current = items.length;
  }, [items.length]);

  return (
    <Popover
      width={224}
      trigger={({ toggle }) => (
        <motion.button
          type="button"
          key={hasNew ? "new" : "idle"}
          onClick={() => {
            if (!hasNew) markSeen(sessionId);
            toggle();
          }}
          initial={reduceMotion ? false : { scale: 1 }}
          animate={hasNew ? { scale: [1, 1.3, 0.95, 1.05, 1] } : { scale: 1 }}
          transition={{ duration: 0.4, ease: "easeInOut" }}
          className="relative flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-md text-(--_dk-text-muted) transition-transform duration-100 hover:brightness-110 active:scale-90 active:brightness-90"
          title={`${items.length} notification${items.length !== 1 ? "s" : ""}`}
        >
          {/* Outline bell (no fill) on a soft circular gradient base — the same
              ShapeBlur halo as the ring, so the bare icon reads clearly against
              the card without a solid glyph. */}
          <ShapeBlur
            shape="radial"
            size={20}
            inset={{ left: 5, top: 5 }}
            strength={6}
            maskSolid={40}
            tintColor="var(--_dk-editor)"
            tint={0.66}
          />
          <Bell
            size={16}
            className={`relative z-10 ${
              hasNew ? "text-(--_dk-accent-hover)" : "text-(--_dk-text-muted)"
            }`}
          />
        </motion.button>
      )}
    >
      {({ close }) => (
        <>
          <div className="max-h-48 overflow-y-auto">
            {items.length === 0 ? (
              <div className="px-3 py-4 text-center text-xs text-(--_dk-text-disabled)">
                No notifications
              </div>
            ) : (
              items.map((it) => (
                <div
                  key={it.id}
                  className={`flex items-start gap-2 border-l-2 px-3 py-2 pr-3 text-[11px] leading-snug ${KIND_BORDER[it.kind]}`}
                >
                  <span className={`mt-[3px] h-1.5 w-1.5 shrink-0 rounded-full ${KIND_DOT[it.kind]}`} />
                  <span className="text-(--_dk-text-secondary)">{it.message}</span>
                </div>
              ))
            )}
          </div>
          <div className="border-t border-(--_dk-line) px-2 py-1.5 flex justify-end">
            <button
              type="button"
              onClick={() => {
                clear(sessionId);
                close();
              }}
              disabled={items.length === 0}
              className="inline-flex items-center gap-1 text-[10px] text-(--_dk-text-muted) hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <Trash size={10} />
              Clear
            </button>
          </div>
        </>
      )}
    </Popover>
  );
}
