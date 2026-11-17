import { beforeEach, expect, test } from "vitest";
import { mockOverview } from "./mock";
import {
  acceptsDraft,
  beginDraftFlip,
  draftPhase,
  draftRecord,
  offerOverview,
  offerPreviewDraft,
  resetDraftRecords,
  settleDraftFlip,
  takeGeneration,
} from "./draftRecord";

beforeEach(() => resetDraftRecords());

const fact = (generation: number, isDraft: boolean, fetchedAt: string | null = null, pending = false) => ({
  generation,
  fetchedAt,
  isDraft,
  pending,
  pendingDraft: pending ? false : null,
});

test("a lower generation and an earlier overview lose to a newer ready fact", () => {
  const ready = fact(4, false, "2026-11-17T12:00:00Z");
  expect(acceptsDraft(ready, fact(3, true, "2026-11-17T12:00:01Z"))).toBe(false);
  expect(acceptsDraft(ready, fact(9, true, "2026-11-17T11:00:00Z"))).toBe(false);
  expect(acceptsDraft(ready, fact(9, true, null))).toBe(false);
  expect(acceptsDraft(ready, fact(5, false, "2026-11-17T12:00:01Z"))).toBe(true);
});

test("a pending mark-ready stays until that generation settles", () => {
  const observed = beginDraftFlip(7, false);
  expect(draftPhase(draftRecord(7), true)).toBe("marking");
  expect(offerOverview(7, mockOverview({ is_draft: true, fetched_at: "2026-11-17T13:00:00Z" }), observed + 1)).toBe(false);
  expect(offerPreviewDraft(7, observed + 2)).toBe(false);
  expect(draftPhase(draftRecord(7), true)).toBe("marking");
  expect(settleDraftFlip(7, observed, { ok: true, isDraft: false, fetchedAt: "2026-11-17T12:30:00Z" })).toBe(true);
  expect(draftPhase(draftRecord(7), true)).toBe("ready");
  expect(offerPreviewDraft(7, takeGeneration())).toBe(false);
  expect(draftRecord(7)?.isDraft).toBe(false);
});

test("a failed flip keeps the draft flag", () => {
  const observed = beginDraftFlip(4, false);
  expect(settleDraftFlip(4, observed, { ok: false })).toBe(true);
  expect(draftPhase(draftRecord(4), true)).toBe("draft");
});

test("with no record the row decides, and a preview can reveal a draft", () => {
  expect(draftPhase(null, true)).toBe("draft");
  expect(draftPhase(null, false)).toBe("ready");
  expect(offerPreviewDraft(3, takeGeneration())).toBe(true);
  expect(draftPhase(draftRecord(3), false)).toBe("draft");
});
