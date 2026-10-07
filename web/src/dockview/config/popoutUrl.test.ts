import { describe, expect, it } from "vitest";

import { dockIdFromPopoutUrl, popoutPageUrl } from "./popoutUrl";

const DOCK = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";

describe("popout page url", () => {
  it("builds a popout page with a dock id and reads that id back", () => {
    expect(popoutPageUrl(DOCK)).toBe(`/popout.html?dock=${DOCK}`);
    expect(dockIdFromPopoutUrl(popoutPageUrl(DOCK))).toBe(DOCK);
    expect(dockIdFromPopoutUrl(`http://127.0.0.1:9/popout.html?dock=${DOCK}`)).toBe(DOCK);
  });

  it("rejects urls that are not a single dock query on popout.html", () => {
    expect(dockIdFromPopoutUrl("/popout.html")).toBeNull();
    expect(dockIdFromPopoutUrl("/popout.html?dock=nope")).toBeNull();
    expect(dockIdFromPopoutUrl(`/popout.html?dock=${DOCK}&extra=1`)).toBeNull();
    expect(dockIdFromPopoutUrl(`/index.html?dock=${DOCK}`)).toBeNull();
    expect(dockIdFromPopoutUrl(`/popout.html?dock=${DOCK}#x`)).toBeNull();
    expect(dockIdFromPopoutUrl(null)).toBeNull();
  });
});
