import { describe, expect, it } from "vitest";

import { isLspWarm, relPathFromLspUri, toFileUri } from "./litecodeLsp";
import { useSettingsStore } from "../stores/settingsStore";

describe("toFileUri", () => {
  it("builds absolute file uri from workspace root and relative path", () => {
    expect(toFileUri("/home/proj", "src/main.rs")).toBe(
      "file:///home/proj/src/main.rs",
    );
  });

  it("builds Windows LAP drive path into a valid file uri", () => {
    expect(toFileUri("E:\\litecode", "src/agent/core.rs")).toBe(
      "file:///E:/litecode/src/agent/core.rs",
    );
    expect(toFileUri("e:/litecode", "src/agent/core.rs")).toBe(
      "file:///E:/litecode/src/agent/core.rs",
    );
  });

  it("builds UNC LAP path into a valid file uri", () => {
    expect(toFileUri("\\\\host\\share\\proj", "a.rs")).toBe(
      "file://host/share/proj/a.rs",
    );
  });

  it("rejects Windows verbatim project roots (no silent strip)", () => {
    expect(() => toFileUri("\\\\?\\E:\\litecode", "src/a.rs")).toThrow(/LAP|verbatim/i);
    expect(() => toFileUri("//?/E:/litecode", "src/a.rs")).toThrow(/LAP|verbatim/i);
  });
});

describe("relPathFromLspUri", () => {
  it("strips workspace root from lsp file uri", () => {
    expect(
      relPathFromLspUri(
        "file:///home/user/project/src/lsp/mod.rs",
        "/home/user/project",
      ),
    ).toBe("src/lsp/mod.rs");
  });

  it("strips Windows LAP root from lsp file uri", () => {
    expect(
      relPathFromLspUri(
        "file:///E:/litecode/src/agent/core.rs",
        "E:\\litecode",
      ),
    ).toBe("src/agent/core.rs");
  });

  it("rejects verbatim project roots", () => {
    expect(() =>
      relPathFromLspUri("file:///E:/litecode/a.rs", "\\\\?\\E:\\litecode"),
    ).toThrow(/LAP|verbatim/i);
  });
});

describe("isLspWarm", () => {
  it("requires warm engine state", () => {
    useSettingsStore.setState({
      engineStatuses: {
        lsp: { desired: true, state: "warm" },
      },
    });
    expect(isLspWarm()).toBe(true);

    useSettingsStore.setState({
      engineStatuses: {
        lsp: { desired: true, state: "warming" },
      },
    });
    expect(isLspWarm()).toBe(false);
  });
});
