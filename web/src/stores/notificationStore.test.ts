import { describe, expect, it } from "vitest";

import { sessionNotificationItems, useNotificationStore } from "./notificationStore";

describe("notificationStore session isolation", () => {
  it("keeps hook items in the originating session bucket", () => {
    useNotificationStore.setState({ bySession: new Map() });
    const add = useNotificationStore.getState().add;
    add("sess-a", "Hook: Continue");
    add("sess-b", "Hook: Block");

    expect(sessionNotificationItems(useNotificationStore.getState().bySession, "sess-a").map((i) => i.message)).toEqual([
      "Hook: Continue",
    ]);
    expect(sessionNotificationItems(useNotificationStore.getState().bySession, "sess-b").map((i) => i.message)).toEqual([
      "Hook: Block",
    ]);
    expect(sessionNotificationItems(useNotificationStore.getState().bySession, "sess-c")).toHaveLength(0);

    useNotificationStore.getState().clear("sess-a");
    expect(sessionNotificationItems(useNotificationStore.getState().bySession, "sess-a")).toHaveLength(0);
    expect(sessionNotificationItems(useNotificationStore.getState().bySession, "sess-b")).toHaveLength(1);
  });
});
