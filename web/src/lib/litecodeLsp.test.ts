import { describe, expect, it } from "vitest";

import {
  dropWorkspaceLsp,
  isLspServerReady,
  isLspWarm,
  monacoChangeToLsp,
  monacoCompletionTriggerToLsp,
  parseDiagnosticsSnapshot,
  parseDidChangeAck,
  relPathFromLspUri,
  shouldApplyPublishedDiagnostics,
  toFileUri,
} from "./litecodeLsp";
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

describe("monacoCompletionTriggerToLsp", () => {
  it("maps Monaco 0/1/2 to LSP 1/2/3", () => {
    expect(monacoCompletionTriggerToLsp(0)).toBe(1);
    expect(monacoCompletionTriggerToLsp(1)).toBe(2);
    expect(monacoCompletionTriggerToLsp(2)).toBe(3);
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

describe("isLspServerReady", () => {
  it("is false on hub Warm with no running instance", () => {
    dropWorkspaceLsp();
    useSettingsStore.setState({
      engineStatuses: {
        lsp: { desired: true, state: "warm" },
      },
      lspServers: [],
    });
    expect(isLspWarm()).toBe(true);
    expect(isLspServerReady()).toBe(false);
  });

  it("is true when engines detail reports a running instance", () => {
    dropWorkspaceLsp();
    useSettingsStore.setState({
      engineStatuses: {
        lsp: { desired: true, state: "warm" },
      },
      lspServers: [
        {
          command: "rust-analyzer",
          project_root: "/proj",
          state: "running",
          index_settled: true,
          restart_count: 0,
        },
      ],
    });
    expect(isLspServerReady()).toBe(true);
  });
});

describe("parseDiagnosticsSnapshot", () => {
  it("treats a missing version cover as silence, not an error list", () => {
    const snap = parseDiagnosticsSnapshot({
      rev: 2,
      fresh: false,
      server_ready: true,
      diagnostics: [{ message: "stale v1", severity: 1 }],
    });
    expect(snap.fresh).toBe(false);
    expect(snap.serverReady).toBe(true);
    expect(snap.rev).toBe(2);
  });

  it("does not treat a raw diagnostic array as a fresh snapshot", () => {
    const snap = parseDiagnosticsSnapshot([{ message: "old shape" }]);
    expect(snap.fresh).toBe(false);
    expect(snap.serverReady).toBe(false);
    expect(snap.diagnostics).toEqual([]);
  });

  it("paints only when fresh and server_ready", () => {
    const snap = parseDiagnosticsSnapshot({
      rev: 3,
      fresh: true,
      server_ready: true,
      diagnostics: [{ message: "real" }],
    });
    expect(snap.fresh && snap.serverReady).toBe(true);
    expect(snap.diagnostics).toEqual([{ message: "real" }]);
  });
});

describe("parseDidChangeAck", () => {
  it("reads rev/version without treating diagnostics as the payload", () => {
    const ack = parseDidChangeAck({
      rev: 4,
      version: 2,
      server_ready: true,
      diagnostics: [{ message: "must ignore" }],
    });
    expect(ack).toEqual({ rev: 4, version: 2, serverReady: true });
  });

  it("does not treat a snapshot-shaped silence as ready", () => {
    expect(parseDidChangeAck({ fresh: false, diagnostics: [] })).toEqual({
      rev: null,
      version: null,
      serverReady: false,
    });
  });
});

describe("shouldApplyPublishedDiagnostics", () => {
  it("keeps markers when a versioned publish is older than the sent document", () => {
    expect(shouldApplyPublishedDiagnostics(1, 3)).toBe(false);
    expect(shouldApplyPublishedDiagnostics(3, 3)).toBe(true);
    expect(shouldApplyPublishedDiagnostics(4, 3)).toBe(true);
  });

  it("applies unversioned publishes", () => {
    expect(shouldApplyPublishedDiagnostics(null, 3)).toBe(true);
    expect(shouldApplyPublishedDiagnostics(undefined, 3)).toBe(true);
  });
});

describe("monacoChangeToLsp", () => {
  it("converts 1-based Monaco ranges to 0-based LSP edits", () => {
    expect(
      monacoChangeToLsp({
        range: {
          startLineNumber: 2,
          startColumn: 3,
          endLineNumber: 2,
          endColumn: 5,
        },
        text: "xy",
      }),
    ).toEqual({
      range: {
        start: { line: 1, character: 2 },
        end: { line: 1, character: 4 },
      },
      text: "xy",
    });
  });
});
