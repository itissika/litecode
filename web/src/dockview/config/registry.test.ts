import { describe, expect, it } from "vitest";

import { PANEL_KINDS } from "../workbench/kinds";
import { panelComponents, tabComponents } from "./registry";

/**
 * DockviewReact only falls back to its own tab when `tabComponent` is
 * undefined; a name that is not in `tabComponents` throws while `addPanel`
 * runs and leaves an empty group behind. Keep every declared kind resolvable.
 */
describe("dockview registry", () => {
  it("registers a content component for every panel kind", () => {
    for (const kind of PANEL_KINDS) {
      expect(panelComponents[kind.component], kind.component).toBeTypeOf("function");
    }
  });

  it("registers a tab component for every panel kind", () => {
    for (const kind of PANEL_KINDS) {
      expect(tabComponents[kind.tabComponent], kind.tabComponent).toBeTypeOf("function");
    }
  });
});
