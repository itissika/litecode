import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { apiFetch } from "../api/auth";
import { ImageThumb } from "./ImageThumb";

vi.mock("../api/auth", () => ({
  apiFetch: vi.fn(),
}));

const NAME = "b".repeat(64);

beforeEach(() => {
  vi.mocked(apiFetch).mockReset();
});

describe("ImageThumb", () => {
  it("shows a cross when the stored file is missing", async () => {
    vi.mocked(apiFetch).mockResolvedValue(new Response(null, { status: 404 }));
    render(<ImageThumb mediaRef={`litecode-media:${NAME}.jpg`} />);
    expect(await screen.findByLabelText("Image unavailable")).toBeTruthy();
  });

  it("shows a cross for a ref that is not a stored image", async () => {
    render(<ImageThumb mediaRef="https://example.com/huge.png" />);
    expect(await screen.findByLabelText("Image unavailable")).toBeTruthy();
  });

  it("paints an unsupported mask over the thumbnail", () => {
    vi.mocked(apiFetch).mockResolvedValue(new Response(null, { status: 404 }));
    render(<ImageThumb mediaRef={`litecode-media:${NAME}.png`} masked />);
    expect(screen.getByText("Unsupported")).toBeTruthy();
  });
});
