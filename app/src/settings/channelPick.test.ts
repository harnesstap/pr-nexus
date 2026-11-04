import { describe, expect, it } from "vitest";
import type { MappingRow } from "../api";
import { applyPick, nameMismatch } from "./channelPick";

const row = (name: string, id: string): MappingRow => ({
  github_org: "acme",
  team_slug: "web",
  team_name: "Web",
  slack_workspace: "acme",
  slack_channel_name: name,
  slack_channel_id: id,
});

describe("applyPick", () => {
  it("sets the channel id and name together", () => {
    const next = applyPick(row("old-channel", "C0ACME0001"), {
      workspace: "acme-labs",
      team_id: "T0ACME0002",
      channel_id: "c0acme0002",
      channel_name: "#new-channel",
    });
    expect(next.slack_channel_id).toBe("C0ACME0002");
    expect(next.slack_channel_name).toBe("new-channel");
    expect(next.slack_workspace).toBe("acme-labs");
    expect(next.slack_team_id).toBe("T0ACME0002");
  });
  it("keeps the row's workspace for an ID entered by hand", () => {
    const next = applyPick(row("old-channel", "C0ACME0001"), { workspace: "", channel_id: "C0ACME0002", channel_name: "" });
    expect(next.slack_workspace).toBe("acme");
    expect(next.slack_channel_name).toBe("");
  });
});

describe("nameMismatch", () => {
  it("flags a saved name that is not the id's channel", () => {
    expect(nameMismatch(row("old-channel", "C0ACME0002"), "new-channel")).toBe("new-channel");
  });
  it("ignores case and #, and a lookup that failed", () => {
    expect(nameMismatch(row("#New-Channel", "C0ACME0002"), "new-channel")).toBeNull();
    expect(nameMismatch(row("old-channel", "C0ACME0002"), null)).toBeNull();
    expect(nameMismatch(row("", "C0ACME0002"), "new-channel")).toBeNull();
  });
});
