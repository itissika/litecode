import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  assertIpcSurface,
  classifyIpcSender,
  exactHttpOrigin,
  isAllowedNavigation,
  type IpcTrustContext,
} from "./ipc-trust";

const hubUrl = "file:///C:/Users/test/AppData/Roaming/Litecode/hub-index.html";

function context(
  activeSurface: IpcTrustContext["activeSurface"] = "hub",
): IpcTrustContext {
  return {
    activeSurface,
    hubFileUrl: hubUrl,
    workbenchOrigin: "https://workbench.example",
  };
}

function sender(url: string) {
  const mainFrame = { url };
  return { mainFrame };
}

describe("Electron IPC trust classification", () => {
  it("accepts only the exact hub file URL", () => {
    const trusted = sender(hubUrl);
    const event = { sender: trusted, senderFrame: trusted.mainFrame };
    assert.deepEqual(classifyIpcSender(event, trusted, context()), {
      trusted: true,
      surface: "hub",
    });

    for (const url of [
      `${hubUrl}?spoof=1`,
      `${hubUrl}#fragment`,
      "file:///C:/Users/test/AppData/Roaming/Litecode/other.html",
    ]) {
      const candidate = sender(url);
      assert.equal(
        classifyIpcSender(
          { sender: candidate, senderFrame: candidate.mainFrame },
          candidate,
          context(),
        ).trusted,
        false,
      );
    }
  });

  it("accepts workbench paths only at the registered exact origin", () => {
    const trusted = sender("https://workbench.example/project?q=1");
    assert.deepEqual(
      classifyIpcSender(
        { sender: trusted, senderFrame: trusted.mainFrame },
        trusted,
        context("workbench"),
      ),
      { trusted: true, surface: "workbench" },
    );

    for (const url of [
      "http://workbench.example/project",
      "https://workbench.example.evil.test/project",
      "https://user@workbench.example/project",
    ]) {
      const candidate = sender(url);
      assert.equal(
        classifyIpcSender(
          { sender: candidate, senderFrame: candidate.mainFrame },
          candidate,
          context("workbench"),
        ).trusted,
        false,
      );
    }
  });

  it("rejects foreign senders, subframes, inactive surfaces, and wrong capabilities", () => {
    const trusted = sender(hubUrl);
    const foreign = sender(hubUrl);
    assert.equal(
      classifyIpcSender(
        { sender: foreign, senderFrame: foreign.mainFrame },
        trusted,
        context(),
      ).trusted,
      false,
    );
    assert.equal(
      classifyIpcSender(
        { sender: trusted, senderFrame: { url: hubUrl } },
        trusted,
        context(),
      ).trusted,
      false,
    );
    assert.throws(
      () =>
        assertIpcSurface(
          { sender: trusted, senderFrame: trusted.mainFrame },
          trusted,
          context(),
          "workbench",
        ),
      /workbench surface required/,
    );
    assert.equal(
      classifyIpcSender(
        { sender: trusted, senderFrame: trusted.mainFrame },
        trusted,
        context("workbench"),
      ).trusted,
      false,
    );
  });
});

describe("navigation and origin policy", () => {
  it("normalizes only credential-free HTTP(S) origins", () => {
    assert.equal(exactHttpOrigin("https://example.com/a"), "https://example.com");
    assert.equal(exactHttpOrigin("http://127.0.0.1:7483/"), "http://127.0.0.1:7483");
    assert.equal(exactHttpOrigin("file:///tmp/a"), null);
    assert.equal(exactHttpOrigin("https://user@example.com/"), null);
  });

  it("keeps hub navigation exact and workbench navigation on its registered origin", () => {
    assert.equal(isAllowedNavigation("hub", hubUrl, context()), true);
    assert.equal(isAllowedNavigation("hub", `${hubUrl}#x`, context()), false);
    assert.equal(
      isAllowedNavigation(
        "workbench",
        "https://workbench.example/another/path",
        context("workbench"),
      ),
      true,
    );
    assert.equal(
      isAllowedNavigation(
        "workbench",
        "https://attacker.example/",
        context("workbench"),
      ),
      false,
    );
  });
});
