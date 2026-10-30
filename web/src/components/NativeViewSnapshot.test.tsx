import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { useNativeViewOcclusionStore } from "../stores/nativeViewOcclusionStore";
import { NativeViewSnapshot } from "./NativeViewSnapshot";

afterEach(() => {
  cleanup();
  useNativeViewOcclusionStore.getState().clearAll();
});

describe("NativeViewSnapshot", () => {
  it("renders the captured frame without stealing the placeholder out of the host", () => {
    useNativeViewOcclusionStore
      .getState()
      .setPlaceholder("browser", "b1", "data:image/jpeg;base64,c25hcHNob3Q=");

    const { container } = render(<NativeViewSnapshot kind="browser" id="b1" />);
    const image = container.querySelector("img");
    expect(image?.getAttribute("src")).toContain("data:image/jpeg");
    expect(container.querySelector("[data-native-view-placeholder='browser']")).toBeTruthy();
  });

  it("keeps a blocker with a hint when the capture is empty", () => {
    useNativeViewOcclusionStore.getState().setPlaceholder("app", "a1", "");

    const { container } = render(<NativeViewSnapshot kind="app" id="a1" />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("[data-native-view-placeholder='app']")?.textContent?.length).toBeGreaterThan(0);
  });

  it("renders nothing when the view is not occluded", () => {
    const { container } = render(<NativeViewSnapshot kind="browser" id="missing" />);
    expect(container.firstChild).toBeNull();
  });
});
