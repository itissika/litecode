import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BotInfo, SessionInfo } from "../api/types";
import { BotItem } from "./BotItem";

function bot(patch: Partial<BotInfo> = {}): BotInfo {
  return {
    id: "b1",
    name: "Researcher",
    description: "finds papers",
    body: { system_prompt: "", tools: {}, allowed_subagents: [] },
    created_at: 1,
    session_id: "s1",
    folder_id: null,
    sort_order: 1,
    ...patch,
  };
}

function session(patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id: "s1",
    project: "/p",
    updated_at: 1,
    preview: "session preview",
    running: false,
    turn: null,
    agent_id: "bot_b1",
    api_model_id: "m",
    ...patch,
  };
}

afterEach(cleanup);

describe("BotItem", () => {
  it("uses the shared row for open, rename, and context menu", () => {
    const onOpen = vi.fn();
    const onRename = vi.fn();
    const onContextMenu = vi.fn();
    const onStartRename = vi.fn();
    const { rerender } = render(
      <BotItem
        bot={bot()}
        session={session()}
        now={2}
        onOpen={onOpen}
        onDelete={vi.fn()}
        onContextMenu={onContextMenu}
        onStartRename={onStartRename}
        onRename={onRename}
        onRenameCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByText("Researcher"));
    expect(onOpen).toHaveBeenCalledWith("b1");
    fireEvent.doubleClick(screen.getByText("Researcher"));
    expect(onStartRename).toHaveBeenCalled();

    rerender(
      <BotItem
        bot={bot()}
        session={session()}
        now={2}
        renaming
        onOpen={onOpen}
        onDelete={vi.fn()}
        onContextMenu={onContextMenu}
        onRename={onRename}
        onRenameCancel={vi.fn()}
      />,
    );
    const input = screen.getByRole("textbox", { name: "Bot name" });
    fireEvent.change(input, { target: { value: "Reviewer" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onRename).toHaveBeenCalledWith("Reviewer");

    rerender(
      <BotItem
        bot={bot()}
        session={session()}
        now={2}
        onOpen={onOpen}
        onDelete={vi.fn()}
        onContextMenu={onContextMenu}
        onRename={onRename}
        onRenameCancel={vi.fn()}
      />,
    );
    fireEvent.contextMenu(screen.getByText("Researcher"));
    expect(onContextMenu).toHaveBeenCalled();
  });

  it("blocks delete while the default session is running", () => {
    const onDelete = vi.fn();
    render(
      <BotItem
        bot={bot()}
        session={session({ running: true })}
        now={2}
        onOpen={vi.fn()}
        onDelete={onDelete}
        onRename={vi.fn()}
        onRenameCancel={vi.fn()}
      />,
    );

    const deleteButton = screen.getByRole("button", {
      name: "Stop the bot before deleting it",
    });
    expect((deleteButton as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(deleteButton);
    expect(onDelete).not.toHaveBeenCalled();
  });

  it("shows the bot identity without repeating the session preview", () => {
    render(
      <BotItem
        bot={bot()}
        session={session({ preview: "ordinary preview" })}
        now={2}
        onOpen={vi.fn()}
        onDelete={vi.fn()}
        onRename={vi.fn()}
        onRenameCancel={vi.fn()}
      />,
    );
    expect(screen.getByText("Researcher")).toBeTruthy();
    expect(screen.queryByText("ordinary preview")).toBeNull();
  });
});