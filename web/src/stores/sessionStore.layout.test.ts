import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionFolder, SessionInfo } from "../api/types";
import { useConnectionStore } from "./connectionStore";
import { useSessionStore } from "./sessionStore";

function session(id: string, patch: Partial<SessionInfo> = {}): SessionInfo {
  return {
    id,
    project: "/p",
    updated_at: 1,
    preview: id,
    running: false,
    turn: null,
    agent_id: "default",
    api_model_id: "m",
    ...patch,
  };
}

function folder(id: string, name: string, sortOrder: number): SessionFolder {
  return { id, name, sort_order: sortOrder };
}

let sendRpc: ReturnType<typeof vi.fn>;
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  sendRpc = vi.fn(async () => ({}));
  useConnectionStore.setState({ state: "connected", sendRpc } as never);
  useSessionStore.setState({
    sessions: [],
    folders: [],
    deletedSessionIds: {},
    sessionListError: null,
  } as never);
});

describe("session layout store", () => {
  it("renames optimistically and trims the wire title", () => {
    useSessionStore.setState({
      sessions: [session("s1", { title: "Old" })],
    } as never);

    useSessionStore.getState().renameSession("s1", "  New title  ");

    expect(useSessionStore.getState().sessions[0].title).toBe("New title");
    expect(sendRpc).toHaveBeenCalledWith("session/rename", {
      id: "s1",
      title: "New title",
    });
  });

  it("rolls a failed rename back to the previous title", async () => {
    sendRpc.mockRejectedValueOnce(new Error("nope"));
    useSessionStore.setState({
      sessions: [session("s1", { title: "Old" })],
    } as never);

    useSessionStore.getState().renameSession("s1", "New");
    expect(useSessionStore.getState().sessions[0].title).toBe("New");

    await flush();
    expect(useSessionStore.getState().sessions[0].title).toBe("Old");
  });

  it("moves optimistically and can clear the folder", () => {
    useSessionStore.setState({
      sessions: [session("s1", { folder_id: "f1" })],
    } as never);

    useSessionStore.getState().moveSession("s1", "f2");
    expect(useSessionStore.getState().sessions[0].folder_id).toBe("f2");
    expect(sendRpc).toHaveBeenCalledWith("session/move", {
      id: "s1",
      folder_id: "f2",
    });

    useSessionStore.getState().moveSession("s1", null);
    expect(useSessionStore.getState().sessions[0].folder_id).toBeNull();
  });

  it("deleting a folder ungroups its sessions and rolls back on failure", async () => {
    sendRpc.mockRejectedValueOnce(new Error("nope"));
    useSessionStore.setState({
      sessions: [
        session("s1", { folder_id: "f1" }),
        session("s2", { folder_id: "f2" }),
      ],
      folders: [folder("f1", "Work", 0), folder("f2", "Other", 1)],
    } as never);

    useSessionStore.getState().deleteFolder("f1");
    expect(useSessionStore.getState().folders.map((f) => f.id)).toEqual(["f2"]);
    expect(useSessionStore.getState().sessions[0].folder_id).toBeNull();
    expect(useSessionStore.getState().sessions[1].folder_id).toBe("f2");

    await flush();
    expect(useSessionStore.getState().folders.map((f) => f.id)).toEqual([
      "f1",
      "f2",
    ]);
    expect(useSessionStore.getState().sessions[0].folder_id).toBe("f1");
  });

  it("onSessionList sorts folders and preserves them when the field is absent", () => {
    useSessionStore
      .getState()
      .onSessionList(
        [session("s1")],
        [folder("f2", "B", 2), folder("f1", "A", 1)],
      );
    expect(useSessionStore.getState().folders.map((f) => f.id)).toEqual([
      "f1",
      "f2",
    ]);

    useSessionStore.getState().onSessionList([session("s1")]);
    expect(useSessionStore.getState().folders.map((f) => f.id)).toEqual([
      "f1",
      "f2",
    ]);
  });

  it("createFolder adds the reply id only when the list push missed it", async () => {
    sendRpc.mockResolvedValueOnce({ folder_id: "f9" });
    useSessionStore.getState().createFolder("New");
    await flush();
    expect(useSessionStore.getState().folders.map((f) => f.id)).toEqual(["f9"]);
  });
});
