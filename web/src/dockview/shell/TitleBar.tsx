import { useEffect, useState, type CSSProperties } from "react";
import { MenuBar } from "../menu/MenuBar";
import { Logo } from "../../components/Logo";
import { useServerVersionTags, VersionTags } from "../../components/VersionTags";

interface TitleBarProps {
  onMenuAction?: (item: string) => void;
  sessionMode?: "local" | "remote";
}

export function TitleBar({ onMenuAction, sessionMode = "local" }: TitleBarProps) {
  const [maximized, setMaximized] = useState(false);
  const hasWindowChrome = typeof window.litecode?.windowClose === "function";
  const versionTags = useServerVersionTags();

  useEffect(() => {
    if (!hasWindowChrome) return;
    void window.litecode?.windowIsMaximized?.().then(setMaximized);
  }, [hasWindowChrome]);

  const dragStyle = {
    WebkitAppRegion: "drag",
    background: "var(--_dk-header)",
    borderBottom: "1px solid var(--_dk-line-visible)",
    color: "var(--_dk-text-muted)",
  } as CSSProperties;

  const noDrag = { WebkitAppRegion: "no-drag" } as CSSProperties;

  return (
    <div className="flex h-[32px] shrink-0 select-none" style={dragStyle}>
      <div className="flex items-center">
        <img
          src="/icon.png"
          alt=""
          className="ml-3 h-[14px] w-[14px] shrink-0"
          draggable={false}
        />
        <Logo size="sm" animated={false} />
        <span style={noDrag}>
          <VersionTags {...versionTags} size="xs" className="ml-2 mr-1" />
        </span>
        <MenuBar onAction={onMenuAction} sessionMode={sessionMode} />
        {sessionMode === "remote" ? (
          <span
            className="ml-2 px-1.5 text-[10px] uppercase tracking-wide"
            style={{
              ...noDrag,
              color: "var(--_dk-text-primary)",
              background: "var(--_dk-line-visible)",
              borderRadius: 2,
              lineHeight: "16px",
            }}
            title="Connected to a remote litecode serve (no local sidecar)"
          >
            Remote
          </span>
        ) : null}
      </div>
      {hasWindowChrome ? (
        <div className="ml-auto flex items-center" style={noDrag}>
          <button
            type="button"
            aria-label="Minimize"
            className="px-3 py-0 h-[32px] text-xs hover:brightness-125 active:brightness-75"
            onClick={() => void window.litecode?.windowMinimize?.()}
          >
            <svg width="10" height="10" viewBox="0 0 10 10">
              <rect y="4" width="10" height="1.5" fill="currentColor" />
            </svg>
          </button>
          <button
            type="button"
            aria-label={maximized ? "Restore" : "Maximize"}
            className="px-3 py-0 h-[32px] text-xs hover:brightness-125 active:brightness-75"
            onClick={() => {
              void window.litecode?.windowMaximizeToggle?.().then(setMaximized);
            }}
          >
            {maximized ? (
              <svg width="10" height="10" viewBox="0 0 10 10">
                <path
                  d="M2 3h5v5H2V3zm1-1h5v5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                />
              </svg>
            ) : (
              <svg width="10" height="10" viewBox="0 0 10 10">
                <rect
                  x="1"
                  y="1"
                  width="8"
                  height="8"
                  rx="0.5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.5"
                />
              </svg>
            )}
          </button>
          <button
            type="button"
            aria-label="Close"
            className="px-3 py-0 h-[32px] text-xs lc-titlebar-close active:brightness-75"
            onClick={() => void window.litecode?.windowClose?.()}
          >
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path
                d="M1 1l8 8M9 1L1 9"
                stroke="currentColor"
                strokeWidth="1.5"
              />
            </svg>
          </button>
        </div>
      ) : (
        <div className="ml-auto" />
      )}
    </div>
  );
}
