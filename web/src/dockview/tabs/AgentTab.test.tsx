import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { IDockviewPanelProps } from "dockview-react";

import type { SessionInfo } from "../../api/types";
import { useSessionStore } from "../../stores/sessionStore";
import { AgentTab } from "./AgentTab";

const ID = "abcdef12-3456-7890-abcd-ef1234567890";

function panelProps(params: Record<string, unknown>): IDockviewPanelProps {
  return {
    params,
    api: { close: vi.fn() },
  } as unknown as IDockviewPanelProps;
}

/** Seed the session list with `ID` (or an empty list for an unseen session). */
function seed(session: Partial<SessionInfo> | null): void {
  useSessionStore.setState({
    sessions:
      session === null
        ? []
        : [
            {
              id: ID,
              project: "/p",
              updated_at: 0,
              preview: "",
              running: false,
              turn: null,
              agent_id: "default",
              api_model_id: "m",
              ...session,
            },
          ],
  });
}

afterEach(() => {
  cleanup();
  useSessionStore.setState({ sessions: [] });
});

describe("AgentTab title", () => {
  it("shows NEW for a freshly created root with no preview yet", () => {
    seed(null);
    render(
      <AgentTab {...panelProps({ sessionId: ID, sessionKind: "root" })} />,
    );

    expect(screen.getByText("NEW")).toBeTruthy();
  });

  it("shows the live preview once the session has one", () => {
    seed({ preview: "fix the tab title" });
    render(
      <AgentTab {...panelProps({ sessionId: ID, sessionKind: "root" })} />,
    );

    expect(screen.getByText("fix the tab title")).toBeTruthy();
    expect(screen.queryByText("NEW")).toBeNull();
  });

  it("keeps the short id for a subagent without a preview", () => {
    seed({ parent_session_id: "root", preview: "" });
    render(<AgentTab {...panelProps({ sessionId: ID })} />);

    expect(screen.getByText(ID.slice(0, 8))).toBeTruthy();
    expect(screen.queryByText("NEW")).toBeNull();
  });

  it("keeps the short id for an unclassified id with no root intent", () => {
    seed(null);
    render(<AgentTab {...panelProps({ sessionId: ID })} />);

    expect(screen.getByText(ID.slice(0, 8))).toBeTruthy();
    expect(screen.queryByText("NEW")).toBeNull();
  });
});
