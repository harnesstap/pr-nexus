import type { Api, PrDetail, ReviewPreview, ReviewTarget, TargetChoice } from "../api";
import { isSendable } from "../api";
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
  timer = setTimeout(() => {
    timer = undefined;
    void fill(api, prId, detail(), gen);
  }, PREFETCH_MS);
}

async function fill(api: Api, prId: number, known: PrDetail | null, gen: number): Promise<void> {
  const detail = known ?? (await api.prDetail(prId).catch(() => null));
  if (gen !== generation || !detail) {
    return;
  }
  if (needsGroupSends(detail.owner_groups)) {
    return;
  }
  const teams = detail.mandatory_teams.length > 0 ? detail.mandatory_teams : detail.candidate_teams;
  const choices = await api.reviewTargets(prId, teams, true).catch(() => [] as TargetChoice[]);
  if (gen !== generation) {
    return;
  }
  const choice = choices.find((item) => item.source !== "unresolved" && isSendable(item.target)) ?? null;
  const preview = choice ? await api.reviewPreview(prId, choice.target, true).catch(() => null) : null;
  if (gen !== generation) {
    return;
  }
  seed = { prId, teams, choices, choice, target: choice?.target ?? null, preview };
}
