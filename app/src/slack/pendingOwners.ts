import type { OwnerGroup, PrDetail, TargetChoice } from "../api";
import { isSendable } from "../api";

/** `org/slug`, lowercased, without a leading `@`. */
export function teamKey(name: string): string {
  return name.trim().replace(/^@/, "").toLowerCase();
}

export interface OwnerSlackEvidence {
  approved_teams?: string[];
  requested_teams?: string[];
  owner_groups?: Pick<OwnerGroup, "teams" | "satisfied">[];
}

function covered(team: string, among: string[], evidence: OwnerSlackEvidence): boolean {
  const key = teamKey(team);
  if ((evidence.approved_teams ?? []).some((other) => teamKey(other) === key)) {
    return true;
  }
  const groups = (evidence.owner_groups ?? []).filter((group) => group.teams.some((other) => teamKey(other) === key));
  if (groups.length > 0 && groups.every((group) => group.satisfied)) {
    return true;
  }
  const requested = new Set((evidence.requested_teams ?? []).map(teamKey));
  const stillWaiting = among.some((other) => requested.has(teamKey(other)));
  return stillWaiting && !requested.has(key);
}

/** Teams still waiting for a review. If every team is covered, or none are, the original list. */
export function pendingOwnerTeams(teams: string[], evidence: OwnerSlackEvidence): string[] {
  if (teams.length <= 1) {
    return [...teams];
  }
  const pending = teams.filter((team) => !covered(team, teams, evidence));
  return pending.length === 0 ? [...teams] : pending;
}

/** Pending teams first, then already-covered teams. Keeps the original order when that split is empty. */
export function preferPendingOwnerTeams(teams: string[], evidence: OwnerSlackEvidence): string[] {
  if (teams.length <= 1) {
    return [...teams];
  }
  const pending = teams.filter((team) => !covered(team, teams, evidence));
  const done = teams.filter((team) => covered(team, teams, evidence));
  if (pending.length === 0 || done.length === 0) {
    return [...teams];
  }
  return [...pending, ...done];
}

/** Mandatory owners, or candidates, with pending teams first. */
export function suggestionTeams(
  detail: Pick<PrDetail, "mandatory_teams" | "candidate_teams"> & OwnerSlackEvidence,
): string[] {
  const teams = detail.mandatory_teams.length > 0 ? detail.mandatory_teams : detail.candidate_teams;
  return preferPendingOwnerTeams(teams, detail);
}

function choiceIsPending(choice: TargetChoice, pending: Set<string>, pendingChannels: Set<string>): boolean {
  if (choice.teams.some((team) => pending.has(teamKey(team)))) {
    return true;
  }
  if (choice.teams.some((team) => teamKey(team).includes("/"))) {
    return false;
  }
  return choice.teams.length === 0 && pendingChannels.has(choice.target.channel_id);
}

/** Pending code-owner channels first. Already-covered channels follow, in their original order. */
export function rankTargetChoices(choices: TargetChoice[], pendingTeams: string[]): TargetChoice[] {
  const pending = new Set(pendingTeams.map(teamKey));
  if (pending.size === 0) {
    return [...choices];
  }
  const pendingChannels = new Set(
    choices.filter((choice) => choice.teams.some((team) => pending.has(teamKey(team))) && choice.target.channel_id).map((choice) => choice.target.channel_id),
  );
  const first = choices.filter((choice) => choiceIsPending(choice, pending, pendingChannels));
  const rest = choices.filter((choice) => !choiceIsPending(choice, pending, pendingChannels));
  if (first.length === 0 || rest.length === 0) {
    return [...choices];
  }
  return [...first, ...rest];
}

/**
 * The channel to preselect: a sendable pending code-owner channel when one
 * exists. If every team is already covered, the first sendable choice, as before.
 */
export function firstSendableChoice(choices: TargetChoice[], teams: string[], evidence: OwnerSlackEvidence): TargetChoice | undefined {
  const sendable = (choice: TargetChoice) => choice.source !== "unresolved" && isSendable(choice.target);
  const stillPending = teams.filter((team) => !covered(team, teams, evidence));
  const split = stillPending.length > 0 && stillPending.length < teams.length;
  const pending = split ? stillPending : teams;
  const ranked = rankTargetChoices(choices, pending);
  if (split) {
    const pendingKeys = new Set(stillPending.map(teamKey));
    const pendingChannels = new Set(
      choices.filter((choice) => choice.teams.some((team) => pendingKeys.has(teamKey(team))) && choice.target.channel_id).map((choice) => choice.target.channel_id),
    );
    return ranked.find((choice) => sendable(choice) && choiceIsPending(choice, pendingKeys, pendingChannels));
  }
  return ranked.find(sendable);
}
