import { describe, expect, it } from "vitest";
import type { MappingRow } from "../api";
import { staleChannelId } from "./MappingSettings";

const row = (name: string, id: string): MappingRow => ({
  github_org: "acme",
  team_slug: "web",
  team_name: "Web",
  slack_workspace: "acme",
  slack_channel_name: name,
  slack_channel_id: id,
});

describe("staleChannelId", () => {
  it("flags a renamed channel that keeps the old id", () => {
    expect(staleChannelId(row("old-channel", "C0ACME0001"), row("new-channel", "C0ACME0001"))).toBe(true);
  });
  it("accepts a new id or an untouched name", () => {
    expect(staleChannelId(row("old-channel", "C0ACME0001"), row("new-channel", "C0ACME0002"))).toBe(false);
    expect(staleChannelId(row("old-channel", "C0ACME0001"), row("#Old-Channel", "C0ACME0001"))).toBe(false);
    expect(staleChannelId(undefined, row("new-channel", "C0ACME0001"))).toBe(false);
  });
});
