import { useEffect, useMemo, useRef, useState } from "react";

import {
  functionCallOutputText,
  parseFunctionArguments,
} from "../api/adapter";
import type { FunctionCallItem, FunctionCallOutputItem } from "../api/types";
import { formatRunCodeData, runCodeChips } from "../lib/runCodeSummary";
import { FoldCard, FOLDCARD_HEADER_TONE } from "./FoldCard";
import { RunCodeStatus, runCodeChipTotalMs } from "./RunCodeStatus";
import { ToolContentView } from "./ToolContentView";
import { ToolIcon } from "./ToolIcon";
import { deriveToolStatus } from "./toolCallStatus";
import { toolTitle } from "./toolviews/toolTitle";

interface ToolCallCardProps {
  call: FunctionCallItem;
  output?: FunctionCallOutputItem;
  /** UI-only structured tool result data (`run_code` aggregate). */
  data?: unknown;
  streaming?: boolean;
  projectRoot?: string | null;
  onOpenFile?: (path: string) => void;
  /** Owning session id — passed through to `ToolContentView`. */
  sessionId?: string;
  /** Stable FoldCard id for virtual-list remount persistence. */
  foldCardId?: string;
}

export function ToolCallCard({
  call,
  output,
  data,
  streaming = false,
  sessionId,
  foldCardId,
}: ToolCallCardProps) {
  const toolName = call.name;
  const input = parseFunctionArguments(call.arguments);
  const status = deriveToolStatus(output, streaming, call.status);
  const rawOutput = output ? functionCallOutputText(output) : "";
  const inputSummary = toolTitle(toolName, input, rawOutput).summary;
  const runCodeChipsList = useMemo(
    () => (toolName === "run_code" ? runCodeChips(data) : []),
    [toolName, data],
  );
  const runCodeLabel =
    toolName === "run_code" ? formatRunCodeData(data) : null;
  const runCodeChipMs = runCodeChipTotalMs(runCodeChipsList.length);
  const [runCodeAnimating, setRunCodeAnimating] = useState(false);
  const prevHadOutput = useRef(Boolean(output));
  const runCodeAnimTimer = useRef<number | null>(null);

  // run_code: play the chip row once when the result lands, then let the
  // glyph settle. A mounted card that already has output (history, virtual-list
  // remount) stays static.
  useEffect(() => {
    const hadOutput = prevHadOutput.current;
    prevHadOutput.current = Boolean(output);
    if (
      toolName !== "run_code" ||
      !output ||
      hadOutput ||
      runCodeChipsList.length === 0 ||
      runCodeAnimTimer.current !== null
    ) {
      return;
    }
    setRunCodeAnimating(true);
    runCodeAnimTimer.current = window.setTimeout(() => {
      runCodeAnimTimer.current = null;
      setRunCodeAnimating(false);
    }, runCodeChipMs + 80);
  }, [toolName, output, runCodeChipsList.length, runCodeChipMs]);

  useEffect(() => {
    return () => {
      if (runCodeAnimTimer.current !== null) {
        window.clearTimeout(runCodeAnimTimer.current);
      }
    };
  }, []);

  return (
    <FoldCard
      id={foldCardId}
      icon={
        <ToolIcon
          name={toolName}
          status={status}
          streaming={streaming}
          settleDelayMs={toolName === "run_code" ? runCodeChipMs : 0}
        />
      }
      label={
        <span className="flex min-w-0 flex-1 items-center gap-2">
          <span
            className={`${FOLDCARD_HEADER_TONE} shrink-0 font-mono text-dk-xs font-medium text-(--_dk-text-primary)`}
          >
            {toolName}
          </span>
          {toolName === "run_code" ? (
            <RunCodeStatus
              chips={runCodeChipsList}
              streaming={streaming}
              hasOutput={Boolean(output)}
              animate={runCodeAnimating}
              ariaLabel={runCodeLabel}
              fallback={inputSummary}
            />
          ) : (
            inputSummary && (
              <span
                className={`${FOLDCARD_HEADER_TONE} min-w-0 flex-1 truncate text-(--_dk-text-muted)`}
              >
                {inputSummary}
              </span>
            )
          )}
        </span>
      }
      defaultOpen={false}
      streaming={streaming}
    >
      <ToolContentView
        name={toolName}
        status={status}
        input={input}
        output={output}
        callId={call.call_id}
        sessionId={sessionId}
      />
    </FoldCard>
  );
}
