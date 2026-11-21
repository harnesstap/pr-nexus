import { useSyncExternalStore } from "react";
import type { PrOverview } from "../api";

/**
 * The draft flag the detail panel and the Slack dialog both read.
 *
 * Mark ready writes it (optimistically as pending, then with the overview
 * `setPrDraft` returns). A cached preview or an older overview is kept only
 * when it is newer: a higher generation, and not an earlier `fetched_at`.
 * An in-flight flip stays put until that same generation settles, so a late
 * read cannot put the draft warning back.
 */
export interface DraftFact {
  /** Monotonic. Taken when a read or a flip starts. */
  generation: number;
  /** Overview timestamp, when this fact came from one. */
  fetchedAt: string | null;
  isDraft: boolean;
  /** `setPrDraft` for this generation has not returned. */
  pending: boolean;
  /** The draft flag that in-flight call is writing. */
  pendingDraft: boolean | null;
}

export type DraftPhase = "draft" | "ready" | "marking";

let generation = 0;
const records = new Map<number, DraftFact>();
const listeners = new Map<number, Set<() => void>>();

export function takeGeneration(): number {
  generation += 1;
  return generation;
}

export function draftRecord(prId: number): DraftFact | null {
  return records.get(prId) ?? null;
}

export function resetDraftRecords(): void {
  generation = 0;
  records.clear();
  listeners.clear();
}

/** Drops every record, e.g. on a profile switch, where PR ids can collide. The generation keeps counting, so a late read from before loses. */
export function forgetDraftRecords(): void {
  records.clear();
}

function notify(prId: number): void {
  const set = listeners.get(prId);
  if (!set) return;
  for (const listener of set) listener();
}

export function subscribeDraft(prId: number, listener: () => void): () => void {
  let set = listeners.get(prId);
  if (!set) {
    set = new Set();
    listeners.set(prId, set);
  }
  set.add(listener);
  return () => {
    set.delete(listener);
  };
}

/** What the dialog shows. A pending mark-ready hides the settled draft warning. */
export function draftPhase(record: DraftFact | null, rowDraft: boolean): DraftPhase {
  if (record?.pending && record.pendingDraft === false) return "marking";
  if (record && !record.pending) return record.isDraft ? "draft" : "ready";
  if (record?.pending) return record.isDraft ? "draft" : "ready";
  return rowDraft ? "draft" : "ready";
}

/**
 * `next` may replace `current`. A lower generation loses. An earlier
 * `fetched_at` loses. A preview (no timestamp) cannot turn a settled ready
 * fact back into a draft. Pending stays until that flip settles.
 */
export function acceptsDraft(current: DraftFact, next: DraftFact): boolean {
  if (current.pending && !(next.generation === current.generation && !next.pending)) return false;
  if (next.generation < current.generation) return false;
  if (next.fetchedAt && current.fetchedAt && next.fetchedAt < current.fetchedAt) return false;
  if (!next.fetchedAt && next.isDraft && !current.pending && !current.isDraft) return false;
  if (next.generation === current.generation) {
    if (current.pending && !next.pending) return true;
    if (next.fetchedAt && (!current.fetchedAt || next.fetchedAt > current.fetchedAt)) return true;
    return false;
  }
  return true;
}

export function offerDraft(prId: number, next: DraftFact): boolean {
  const current = records.get(prId);
  if (current && !acceptsDraft(current, next)) return false;
  records.set(prId, next);
  notify(prId);
  return true;
}

/** A review preview that still says this PR is a draft. No timestamp of its own. */
export function offerPreviewDraft(prId: number, observed: number): boolean {
  return offerDraft(prId, { generation: observed, fetchedAt: null, isDraft: true, pending: false, pendingDraft: null });
}

export function offerOverview(prId: number, overview: PrOverview, observed: number): boolean {
  return offerDraft(prId, {
    generation: observed,
    fetchedAt: overview.fetched_at,
    isDraft: overview.is_draft,
    pending: false,
    pendingDraft: null,
  });
}

/** The value from `setPrDraft`, when the caller actually got an overview back. */
export function overviewOf(value: unknown): PrOverview | null {
  if (!value || typeof value !== "object") return null;
  const row = value as Partial<PrOverview>;
  if (typeof row.is_draft !== "boolean" || typeof row.fetched_at !== "string" || typeof row.state !== "string") return null;
  return value as PrOverview;
}

/** Mark a draft flip in flight. The settled flag stays until `settleDraftFlip`. */
export function beginDraftFlip(prId: number, draft: boolean): number {
  const observed = takeGeneration();
  const current = records.get(prId);
  records.set(prId, {
    generation: observed,
    fetchedAt: current?.fetchedAt ?? null,
    isDraft: current ? current.isDraft : !draft,
    pending: true,
    pendingDraft: draft,
  });
  notify(prId);
  return observed;
}

export function settleDraftFlip(
  prId: number,
  observed: number,
  result: { ok: true; isDraft: boolean; fetchedAt: string | null } | { ok: false },
): boolean {
  const current = records.get(prId);
  if (!current || current.generation !== observed || !current.pending) return false;
  records.set(
    prId,
    result.ok
      ? { generation: observed, fetchedAt: result.fetchedAt ?? current.fetchedAt, isDraft: result.isDraft, pending: false, pendingDraft: null }
      : { ...current, pending: false, pendingDraft: null },
  );
  notify(prId);
  return true;
}

/** Settle from a `setPrDraft` result. `{}` still counts as the requested flag. */
export function settleDraftCall(prId: number, observed: number, value: unknown, requested: boolean): boolean {
  const overview = overviewOf(value);
  return settleDraftFlip(prId, observed, {
    ok: true,
    isDraft: overview ? overview.is_draft : requested,
    fetchedAt: overview?.fetched_at ?? null,
  });
}

export function useDraftRecord(prId: number): DraftFact | null {
  return useSyncExternalStore(
    (listener) => subscribeDraft(prId, listener),
    () => draftRecord(prId),
    () => draftRecord(prId),
  );
}
