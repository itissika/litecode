import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ImagePreview } from "./ImagePreview";

describe("ImagePreview object URL", () => {
  it("shows the dropped blob instead of reading the workspace", () => {
    render(
      <ImagePreview path="external:C:/shot.png" diskRevision={0} sourceUrl="blob:shot" />,
    );
    const img = screen.getByRole("img", { name: "shot.png" });
    expect(img.getAttribute("src")).toBe("blob:shot");
  });
});
