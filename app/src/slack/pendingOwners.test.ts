import { expect, test } from "vitest";
import type { TargetChoice } from "../api";
import { firstSendableChoice, preferPendingOwnerTeams, rankTargetChoices, suggestionTeams } from "./pendingOwners";

const teamA: TargetChoice = {
  target: { workspace: "acme", channel_id: "C0ACME0001", channel_name: "acme-team-a" },
  teams: ["@acme/team-a"],
  source: "mapping",
};
const teamB: TargetChoice = {
  target: { workspace: "acme", channel_id: "C0ACME0002", channel_name: "acme-team-b" },
  teams: ["acme/team-b"],
  source: "mapping",
};

test("an approved team sorts after a pending team", () => {
  expect(preferPendingOwnerTeams(["acme/team-a", "acme/team-b"], { approved_teams: ["@acme/team-a"] })).toEqual([
    "acme/team-b",
    "acme/team-a",
  ]);
});

test("a team no longer requested sorts after one still requested", () => {
  expect(preferPendingOwnerTeams(["@Acme/team-a", "acme/team-b"], { requested_teams: ["acme/team-b"] })).toEqual([
    "acme/team-b",
    "@Acme/team-a",
  ]);
});

test("a satisfied owner group sorts its team after a pending group", () => {
  expect(
    preferPendingOwnerTeams(["acme/team-a", "acme/team-b"], {
      owner_groups: [
        { teams: ["@acme/team-a"], satisfied: true },
        { teams: ["acme/team-b"], satisfied: false },
      ],
    }),
  ).toEqual(["acme/team-b", "acme/team-a"]);
});

test("all approved keeps the original order", () => {
  expect(preferPendingOwnerTeams(["acme/team-a", "acme/team-b"], { approved_teams: ["acme/team-a", "acme/team-b"] })).toEqual([
    "acme/team-a",
    "acme/team-b",
  ]);
});

test("no coverage signal keeps the original order", () => {
  expect(preferPendingOwnerTeams(["acme/team-a", "acme/team-b"], {})).toEqual(["acme/team-a", "acme/team-b"]);
});

test("mandatory teams fall back to candidates, with pending first", () => {
  expect(
    suggestionTeams({
      mandatory_teams: [],
      candidate_teams: ["acme/team-a", "acme/team-b"],
      approved_teams: ["acme/team-a"],
      requested_teams: ["acme/team-b"],
      owner_groups: [],
    }),
  ).toEqual(["acme/team-b", "acme/team-a"]);
});

test("the default pick is the pending team's channel, even when the approved channel is listed first", () => {
  const picked = firstSendableChoice([teamA, teamB], ["acme/team-a", "acme/team-b"], { approved_teams: ["acme/team-a"] });
  expect(picked?.target.channel_name).toBe("acme-team-b");
  expect(rankTargetChoices([teamA, teamB], ["acme/team-b"]).map((choice) => choice.target.channel_name)).toEqual([
    "acme-team-b",
    "acme-team-a",
  ]);
});

test("when every team approved, the first sendable channel stays the default", () => {
  const picked = firstSendableChoice([teamA, teamB], ["acme/team-a", "acme/team-b"], {
    approved_teams: ["acme/team-a", "acme/team-b"],
  });
  expect(picked?.target.channel_name).toBe("acme-team-a");
});

test("an unresolved pending mapping is not replaced by an approved team's channel", () => {
  const unresolved: TargetChoice = { ...teamB, source: "unresolved", target: { ...teamB.target, channel_id: "" } };
  expect(firstSendableChoice([teamA, unresolved], ["acme/team-a", "acme/team-b"], { approved_teams: ["acme/team-a"] })).toBeUndefined();
});
