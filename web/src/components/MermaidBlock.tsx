import { memo, useEffect, useId, useRef, useState } from "react";

import { loadMermaid, withMindmapTreeLayout } from "../lib/mermaid";
import { getTheme, THEME_CHANGE_EVENT } from "../lib/theme";

const STREAM_DEBOUNCE_MS = 500;

interface MermaidBlockProps {
  code: string;
  streaming?: boolean;
}

type RenderState =
  | { status: "pending" }
  | { status: "loading" }
  | { status: "ready"; svg: string }
  | { status: "error"; message: string };

export const MermaidBlock = memo(function MermaidBlock({
  code,
  streaming = false,
}: MermaidBlockProps) {
  const reactId = useId().replace(/:/g, "");
  const renderSeq = useRef(0);
  const lastRenderedChart = useRef<string | null>(null);
  const lastRenderedTheme = useRef<"dark" | "light">("dark");
  const [debouncedCode, setDebouncedCode] = useState(code);
  const [state, setState] = useState<RenderState>({ status: "pending" });
  const [theme, setTheme] = useState<"dark" | "light">(() =>
    getTheme() === "light" ? "light" : "dark",
  );

  useEffect(() => {
    if (streaming) {
      setState((prev) =>
        prev.status === "pending" ? prev : { status: "pending" },
      );
      return;
    }

    const timer = window.setTimeout(() => {
      setDebouncedCode((prev) => (prev === code ? prev : code));
    }, STREAM_DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
  }, [code, streaming]);

  useEffect(() => {
    if (streaming) return;

    const chart = debouncedCode.trim();
    if (!chart) {
      setState((prev) =>
        prev.status === "pending" ? prev : { status: "pending" },
      );
      return;
    }

    if (chart === lastRenderedChart.current && theme === lastRenderedTheme.current) {
      return;
    }

    const seq = ++renderSeq.current;
    lastRenderedTheme.current = theme;
    setState({ status: "loading" });

    void (async () => {
      try {
        const mermaid = await loadMermaid(theme);
        const renderId = `agent-mermaid-${reactId}-${seq}`;
        const { svg } = await mermaid.render(
          renderId,
          withMindmapTreeLayout(chart),
        );
        if (renderSeq.current !== seq) return;
        lastRenderedChart.current = chart;
        setState({ status: "ready", svg });
      } catch (err) {
        if (renderSeq.current !== seq) return;
        const message =
          err instanceof Error ? err.message : "Failed to render diagram";
        setState({ status: "error", message });
      }
    })();
  }, [debouncedCode, reactId, streaming, theme]);

  useEffect(() => {
    const handler = (e: Event) => {
      setTheme((e as CustomEvent<string>).detail === "light" ? "light" : "dark");
    };
    window.addEventListener(THEME_CHANGE_EVENT, handler);
    return () => window.removeEventListener(THEME_CHANGE_EVENT, handler);
  }, []);

  if (streaming || state.status === "pending") {
    return (
      <div className="agent-mermaid-block mb-2 last:mb-0">
        <div className="agent-mermaid-header">
          <span className="agent-mermaid-label">mermaid</span>
          <span className="agent-mermaid-status">Rendering when complete…</span>
        </div>
        <details className="agent-mermaid-source">
          <summary className="agent-mermaid-source-summary">View source</summary>
          <pre className="agent-mermaid-source-pre">
            <code>{code}</code>
          </pre>
        </details>
      </div>
    );
  }

  if (state.status === "loading") {
    return (
      <div className="agent-mermaid-block mb-2 last:mb-0">
        <div className="agent-mermaid-header">
          <span className="agent-mermaid-label">mermaid</span>
          <span className="agent-mermaid-status">Rendering diagram…</span>
        </div>
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div className="agent-mermaid-block agent-mermaid-block-error mb-2 last:mb-0">
        <div className="agent-mermaid-header">
          <span className="agent-mermaid-label">mermaid</span>
          <span className="agent-mermaid-status">{state.message}</span>
        </div>
        <pre className="agent-mermaid-source-pre">
          <code>{code}</code>
        </pre>
      </div>
    );
  }

  return (
    <div className="agent-mermaid-block mb-2 last:mb-0">
      <div className="agent-mermaid-header">
        <span className="agent-mermaid-label">mermaid</span>
      </div>
      <div
        className="agent-mermaid-wrap"
        dangerouslySetInnerHTML={{ __html: state.svg }}
      />
    </div>
  );
});
