import { expect, test } from "vitest";
import type { BulkCard } from "../api";
import { sameIds, samePlan, snapshot } from "./bumpPlan";

function card(channelId: string, name: string, text: string, prIds: number[]): BulkCard {
  return {
    target: { workspace: "acme", channel_id: channelId, channel_name: name },
    messages: [
      {
        action: "bump",
        prs: prIds.map((id) => ({ pr_id: id, number: id, title: "Title", repo: "acme/widgets" })),
        text,
        permalink: null,
      },
    ],
    warnings: [],
    signed_in: true,
    single_top_level: false,
    can_switch: false,
    fingerprint: channelId,
  };
}

test("same plan requires the same channel, name, text and pull requests", () => {
  const shown = [card("C0ACME0001", "team-reviews", "hello", [1])];
  expect(samePlan(shown, [card("C0ACME0001", "team-reviews", "hello", [1])])).toBe(true);
  expect(samePlan(shown, [card("C0ACME0002", "team-reviews", "hello", [1])])).toBe(false);
  expect(samePlan(shown, [card("C0ACME0001", "other", "hello", [1])])).toBe(false);
  expect(samePlan(shown, [card("C0ACME0001", "team-reviews", "goodbye", [1])])).toBe(false);
  expect(snapshot(shown[0]!).channelId).toBe("C0ACME0001");
});

test("same ids ignores order", () => {
  expect(sameIds([2, 1], [1, 2])).toBe(true);
  expect(sameIds([1], [1, 2])).toBe(false);
});
