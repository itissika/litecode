import { useNativeViewOcclusionStore } from "../stores/nativeViewOcclusionStore";
import { nativeViewKey, type NativeViewKind } from "../lib/overlayGeometry";
import i18n from "../i18n";

/**
 * DOM still frame shown while the native view is hidden. It lives inside the
 * dockview host, so portaled overlays stay above it without any z-index work.
 */
export function NativeViewSnapshot({ kind, id }: { kind: NativeViewKind; id: string }) {
  const placeholder = useNativeViewOcclusionStore(
    (state) => state.placeholders[nativeViewKey(kind, id)],
  );
  if (!placeholder) return null;

  return (
    <div
      data-native-view-placeholder={kind}
      aria-hidden="true"
      className="absolute inset-0 flex items-center justify-center overflow-hidden bg-(--_dk-editor)"
    >
      {placeholder.dataUrl ? (
        <>
          <img
            src={placeholder.dataUrl}
            alt=""
            draggable={false}
            className="h-full w-full select-none object-fill"
          />
          <span className="pointer-events-none absolute right-2 bottom-2 rounded bg-black/50 px-1.5 py-0.5 text-[10px] text-white/80">
            {i18n.t("browser.paused")}
          </span>
        </>
      ) : (
        <span className="px-3 text-center text-xs text-(--_dk-text-muted)">
          {i18n.t("browser.occluded")}
        </span>
      )}
    </div>
  );
}
