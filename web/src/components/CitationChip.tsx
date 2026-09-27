import {
  ArrowSquareOut,
  FileText,
  Function as FunctionIcon,
  Hash,
} from "@phosphor-icons/react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

import { parseFileCitation, type CitationTarget } from "../lib/citationRef";
import {
  peekCitation,
  requestCitation,
  type CitationHit,
} from "../lib/citationResolve";
import { useEditorStore } from "../stores/editorStore";
import { useSessionStore } from "../stores/sessionStore";

const CHIP_CLASS = "agent-citation btn-ghost btn-xs";

export function WebCitationChip({
  href,
  children,
}: {
  href: string;
  children?: ReactNode;
}) {
  return (
    <a
      href={href}
      className={CHIP_CLASS}
      target="_blank"
      rel="noreferrer"
    >
      <ArrowSquareOut size={12} aria-hidden />
      <span className="agent-citation-label">{children}</span>
    </a>
  );
}

export function FileCitationChip({
  href,
  streaming = false,
  children,
}: {
  href: string;
  streaming?: boolean;
  children?: ReactNode;
}) {
  const project = useSessionStore((s) => s.project);
  const target = useMemo(() => parseFileCitation(href), [href]);
  const [hit, setHit] = useState<CitationHit | null>(() =>
    target && project ? (peekCitation(project, target) ?? null) : null,
  );

  useEffect(() => {
    if (!target || !project) {
      setHit(null);
      return;
    }
    const cached = peekCitation(project, target);
    if (cached) {
      setHit(cached);
      return;
    }
    let cancelled = false;
    void requestCitation(project, target, { fresh: !streaming }).then(
      (lookup) => {
        if (cancelled) return;
        setHit(lookup?.exists ? lookup : null);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [project, target, streaming]);

  if (!target || !hit) return <>{children}</>;

  const where = hit.line != null ? `${hit.path}:${hit.line}` : hit.path;
  const open = () => {
    const editor = useEditorStore.getState();
    if (hit.line != null) void editor.openFileAt(hit.path, hit.line);
    else void editor.openFile(hit.path);
  };

  return (
    <button
      type="button"
      className={CHIP_CLASS}
      title={where}
      aria-label={`Open ${where}`}
      onClick={open}
    >
      <CitationIcon target={target} revealed={hit.line != null} />
      <span className="agent-citation-label">{children}</span>
    </button>
  );
}

function CitationIcon({
  target,
  revealed,
}: {
  target: CitationTarget;
  revealed: boolean;
}) {
  if (!revealed || target.kind === "file") {
    return <FileText size={12} aria-hidden />;
  }
  if (target.kind === "symbol") {
    return <FunctionIcon size={12} aria-hidden />;
  }
  return <Hash size={12} aria-hidden />;
}
