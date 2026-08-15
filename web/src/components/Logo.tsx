import { useEffect, useMemo, useState } from "react";

interface LogoProps {
  size: "sm" | "md" | "lg";
  replay?: number;
  splash?: boolean;
  animated?: boolean;
  style?: React.CSSProperties;
}

const FONT_WEIGHT = 600;
const TEXT = "LiteCode";

/** Splash overlay is one-shot per document lifetime (survives React remounts). */
let splashConsumed = false;

/** Mark the splash as already shown so it never covers the workbench (Remotion renders). */
export function skipSplash() {
  splashConsumed = true;
}

function injectStyles() {
  const id = "logo-styles";
  if (document.getElementById(id)) return;
  const el = document.createElement("style");
  el.id = id;
  el.textContent = `
    @font-face {
      font-family: "LogoFont";
      src: url("/fonts/LexendDeca-${FONT_WEIGHT}.ttf") format("truetype");
      font-weight: ${FONT_WEIGHT};
      font-display: block;
    }
    @keyframes logo-in {
      from { opacity: 0; transform: scale(1.8); filter: blur(6px); }
      to   { opacity: 1; transform: scale(1);    filter: blur(0); }
    }
    @keyframes splash-out {
      from { opacity: 1; transform: scale(1);    filter: blur(0); }
      to   { opacity: 0; transform: scale(0.95); filter: blur(4px); }
    }
  `;
  document.head.appendChild(el);
}

function baseLetter(color: string): React.CSSProperties {
  return {
    display: "inline-block",
    fontFamily: "LogoFont",
    fontWeight: FONT_WEIGHT,
    color,
    letterSpacing: "0.025em",
  };
}

export function Logo({ size, replay = 0, splash, animated = true, style: extraStyle }: LogoProps) {
  useEffect(() => { injectStyles(); }, []);

  const fs = size === "sm" ? 12 : size === "lg" ? 80 : 28;
  const color = size === "lg" ? "var(--_dk-text-primary)" : "var(--_dk-text-secondary)";
  const useGradient = size === "lg" && splash;
  const gradientStyle: React.CSSProperties = useGradient
    ? {
        background: `linear-gradient(180deg,
                                    color-mix(in srgb, var(--_dk-text-primary), white 33%) 0%,
                                    var(--_dk-text-primary) 50%,
                                    color-mix(in srgb, var(--_dk-text-primary), black 33%) 100%)`,
        backgroundClip: "text",
        WebkitBackgroundClip: "text",
        WebkitTextFillColor: "transparent",
      }
    : {};

  const letters = useMemo(() => {
    const dur = size === "lg" ? 0.8 : 0.45;
    const stagger = size === "lg" ? 0.02 : 0.05;
    return TEXT.split("").map((ch, i) => (
      <span
        key={animated ? `${replay}-${i}` : i}
        style={{
          ...baseLetter(color),
          ...gradientStyle,
          fontSize: fs,
          ...(animated
            ? {
                animationName: "logo-in",
                animationDuration: `${dur}s`,
                animationDelay: `${i * stagger}s`,
                animationFillMode: "backwards",
                animationTimingFunction: "cubic-bezier(0.16, 1, 0.3, 1)",
              }
            : {}),
        }}
      >
        {ch}
      </span>
    ));
  }, [fs, animated, replay, color]);

  const textEl =
    size === "sm" ? (
      <span className="ml-3 inline-flex items-center leading-none select-none whitespace-nowrap" style={extraStyle}>
        {letters}
      </span>
    ) : (
      <span className="select-none whitespace-nowrap" style={extraStyle}>{letters}</span>
    );

  if (splash) {
    return <SplashOverlay>{textEl}</SplashOverlay>;
  }

  return textEl;
}

function SplashOverlay({ children }: { children: React.ReactNode }) {
  // Once per page load — remounts must not re-cover the workbench.
  const [phase, setPhase] = useState<"showing" | "fading" | "hidden">(() =>
    splashConsumed ? "hidden" : "showing",
  );

  useEffect(() => {
    if (phase !== "showing") return;
    const t = setTimeout(() => setPhase("fading"), 1200);
    return () => clearTimeout(t);
  }, [phase]);

  // Do not rely solely on animationend — Electron can drop it (e.g. mid-theme
  // switch), leaving a full-viewport backdrop-filter that looks like a white
  // screen in light theme.
  useEffect(() => {
    if (phase !== "fading") return;
    const t = setTimeout(() => {
      splashConsumed = true;
      setPhase("hidden");
    }, 700);
    return () => clearTimeout(t);
  }, [phase]);

  if (phase === "hidden") return null;

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 9999,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        background: "color-mix(in srgb, var(--_dk-root) 33%, transparent)",
        // Drop blur while fading — an unfinished fade with backdrop-filter
        // frosts the whole UI (reads as a white screen on light theme).
        backdropFilter: phase === "showing" ? "blur(9px)" : undefined,
        WebkitBackdropFilter: phase === "showing" ? "blur(9px)" : undefined,
        pointerEvents: phase === "fading" ? "none" : undefined,
        animation: phase === "fading" ? "splash-out 0.6s ease-out forwards" : undefined,
      }}
      onAnimationEnd={() => {
        if (phase === "fading") {
          splashConsumed = true;
          setPhase("hidden");
        }
      }}
    >
      {children}
    </div>
  );
}
