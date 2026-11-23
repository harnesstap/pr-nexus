import type { Api, PrDetail, ReviewPreview, ReviewTarget, StoredReviewTeams, TargetChoice } from "../api";
import { firstSendableChoice, pendingOwnerTeams, rankTargetChoices, suggestionTeams } from "./pendingOwners";
import { takeGeneration } from "../detail/draftRecord";
import { needsGroupSends } from "./OwnerGroups";

/** Quiet time after a PR is selected before the cached preview is fetched. */
export const PREFETCH_MS = 250;

export interface ReviewSeed {
  prId: number;
  teams: string[];
  choices: TargetChoice[];
  choice: TargetChoice | null;
  target: ReviewTarget | null;
  preview: ReviewPreview | null;
  /** Draft-record generation captured when this prefetch was scheduled. */
  generation: number;
}

let seed: ReviewSeed | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let generation = 0;

/** The cached preview for `prId`, when the prefetch finished. */
export function peekReviewPrefetch(prId: number): ReviewSeed | null {
  return seed?.prId === prId ? seed : null;
}

/** Drop a scheduled prefetch and ignore one that is already in flight. */
export function cancelReviewPrefetch(): void {
  if (timer !== undefined) {
    clearTimeout(timer);
    timer = undefined;
  }
  generation += 1;
}

/** Cancel, and drop a seed the dialog would otherwise paint. */
export function resetReviewPrefetch(): void {
  cancelReviewPrefetch();
  seed = null;
}

/** Remember a finished prefetch. Tests plant a seed without waiting out the timer. */
export function rememberReviewSeed(next: ReviewSeed | null): void {
  seed = next;
}

/**
 * After {@link PREFETCH_MS}, load cached channel names and a preview.
 * `detail` is read when the timer fires so a panel that loaded owners in
 * that window is reused. Cached reads only: nothing is posted.
 */
export function scheduleReviewPrefetch(api: Api, prId: number, detail: () => PrDetail | null): void {
  if (timer !== undefined) {
    clearTimeout(timer);
  }
  const gen = ++generation;
  // The draft generation is the moment of selection, so a mark-ready that
  // happens while this read is in flight stays newer than the cached preview.
  const observed = takeGeneration();
  timer = setTimeout(() => {
    timer = undefined;
    void fill(api, prId, detail(), gen, observed);
  }, PREFETCH_MS);
}

function detailFromStored(stored: StoredReviewTeams): PrDetail {
  return { mandatory_teams: stored.teams, candidate_teams: [], approved_teams: [], requested_teams: [], owner_groups: stored.owner_groups } as unknown as PrDetail;
}

async function fill(api: Api, prId: number, known: PrDetail | null, gen: number, observed: number): Promise<void> {
  const detail =
    known ??
    (api.reviewTeams
      ? await api.reviewTeams(prId).then(detailFromStored).catch(() => null)
      : await api.prDetail(prId).catch(() => null));
  if (gen !== generation || !detail) {
    return;
  }
  if (needsGroupSends(detail.owner_groups)) {
    return;
  }
  const teams = suggestionTeams(detail);
  const choices = rankTargetChoices(await api.reviewTargets(prId, teams, true).catch(() => [] as TargetChoice[]), pendingOwnerTeams(teams, detail));
  if (gen !== generation) {
    return;
  }
  const choice = firstSendableChoice(choices, teams, detail) ?? null;
  const preview = choice ? await api.reviewPreview(prId, choice.target, true).catch(() => null) : null;
  if (gen !== generation) {
    return;
  }
  seed = { prId, teams, choices, choice, target: choice?.target ?? null, preview, generation: observed };
}
