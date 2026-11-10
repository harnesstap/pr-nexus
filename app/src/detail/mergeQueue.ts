import type { ListRow, MergeQueueInfo, PrOverview } from "../api";
import type { MergeVerdict } from "./status";

const READY = new Set(["clean", "has_hooks", "unstable"]);
const WRITE = new Set(["admin", "maintain", "write"]);

/** A short estimate. GitHub's figure is seconds and may be missing. */
export function etaLabel(seconds: number | null | undefined): string | null {
  if (seconds == null || seconds < 0) {
    return null;
  }
  if (seconds <= 90) {
    return "~1 min";
  }
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) {
    return `~${minutes} min`;
  }
  const hours = Math.round(minutes / 60);
  return hours === 1 ? "~1 hour" : `~${hours} hours`;
}

export interface QueueChip {
  label: string;
  detail: string | null;
  tone: "queue" | "merging" | "failed" | "removed";
}

/** The compact pill on a list row. Null when there is nothing to show. */
export function queueChip(queue: MergeQueueInfo | null | undefined): QueueChip | null {
  if (!queue?.phase) {
    return null;
  }
  switch (queue.phase) {
    case "queued": {
      const parts = ["In queue"];
      if (queue.position != null && queue.position > 0) {
        parts.push(`#${queue.position}`);
      }
      const eta = etaLabel(queue.eta_seconds);
      if (eta) {
        parts.push(eta);
      }
      return { label: parts.join(" · "), detail: null, tone: "queue" };
    }
    case "merging":
      return {
        label: "Merging",
        detail: queue.position != null && queue.position > 0 ? `Position ${queue.position}` : null,
        tone: "merging",
      };
    case "failed":
      return { label: "Queue failed", detail: queue.reason, tone: "failed" };
    case "removed":
      return { label: "Removed from queue", detail: queue.reason, tone: "removed" };
    default: {
      const unreachable: never = queue.phase;
      return unreachable;
    }
  }
}

/** Why this open pull request cannot be added. Null means it can. */
export function enqueueBlock(overview: PrOverview): string | null {
  const queue = overview.merge_queue;
  if (!queue?.required) {
    return "No merge queue on the base branch";
  }
  if (overview.state === "closed") {
    return "Already closed";
  }
  if (overview.state === "merged") {
    return "Already merged";
  }
  if (overview.is_draft || overview.merge_state === "draft") {
    return "Still a draft";
  }
  if (queue.phase === "queued" || queue.phase === "merging") {
    return "Already in the merge queue";
  }
  if (overview.viewer.permission !== null && !WRITE.has(overview.viewer.permission)) {
    return "You need write access to add this to the merge queue.";
  }
  if (overview.head_sha === "") {
    return "Head commit is unknown";
  }
  if (overview.mergeable === "conflicting" || overview.merge_state === "dirty") {
    return "Has merge conflicts";
  }
  if (overview.merge_state === "behind") {
    return "Branch is behind";
  }
  if (!READY.has(overview.merge_state)) {
    return "Not ready to merge";
  }
  return null;
}

export function canDequeue(overview: PrOverview): boolean {
  const phase = overview.merge_queue?.phase;
  return phase === "queued" || phase === "merging" || phase === "failed";
}

/** The same skip reasons, from the fields a list row has. */
export function rowEnqueueSkip(row: ListRow): string | null {
  const queue = row.merge_queue;
  if (!queue?.required) {
    return "No merge queue on the base branch";
  }
  if (row.state === "closed") {
    return "Already closed";
  }
  if (row.state === "merged") {
    return "Already merged";
  }
  if (row.is_draft) {
    return "Still a draft";
  }
  if (queue.phase === "queued" || queue.phase === "merging") {
    return "Already in the merge queue";
  }
  if (row.conflicts || row.merge_state === "dirty") {
    return "Has merge conflicts";
  }
  if (row.merge_state === "behind") {
    return "Branch is behind";
  }
  if (row.merge_state != null && row.merge_state !== "" && !READY.has(row.merge_state)) {
    return "Not ready to merge";
  }
  return null;
}

export interface EnqueuePlan {
  add: ListRow[];
  skip: { row: ListRow; reason: string }[];
}

export function enqueuePlan(rows: ListRow[]): EnqueuePlan {
  const add: ListRow[] = [];
  const skip: { row: ListRow; reason: string }[] = [];
  for (const row of rows) {
    const reason = rowEnqueueSkip(row);
    if (reason) {
      skip.push({ row, reason });
    } else {
      add.push(row);
    }
  }
  return { add, skip };
}

/** Rewrites the merge verdict when the base branch requires the queue. */
export function applyQueueVerdict(overview: PrOverview, verdict: MergeVerdict): MergeVerdict {
  const queue = overview.merge_queue;
  if (!queue?.required || overview.state !== "open") {
    return verdict;
  }
  if (queue.phase === "queued") {
    return {
      can: false,
      tone: "good",
      title: "In the merge queue",
      reason: queueLine(queue),
      blockers: [],
    };
  }
  if (queue.phase === "merging") {
    return {
      can: false,
      tone: "good",
      title: "Merging from the queue",
      reason: "GitHub is merging this pull request from the queue.",
      blockers: [],
    };
  }
  if (queue.phase === "failed") {
    return {
      can: false,
      tone: "warn",
      title: "Queue failed",
      reason: queue.reason ?? "GitHub could not merge it from the queue.",
      blockers: [],
    };
  }
  if (verdict.can && overview.merge_state === "blocked" && overview.viewer.can_merge_as_admin) {
    return {
      can: false,
      tone: "warn",
      title: "You can't merge",
      reason: "This base branch uses a merge queue, so it can't be merged directly.",
      blockers: verdict.blockers,
    };
  }
  if (verdict.can) {
    const removed = queue.phase === "removed" && queue.reason ? `Removed from the queue. ${queue.reason}` : null;
    return {
      can: false,
      tone: "good",
      title: "Ready for the merge queue",
      reason: removed ?? "This base branch uses a merge queue.",
      blockers: [],
    };
  }
  if (queue.phase === "removed" && queue.reason) {
    return { ...verdict, reason: `Removed from the queue. ${queue.reason}` };
  }
  return verdict;
}

function queueLine(queue: MergeQueueInfo): string {
  const where = queue.position != null && queue.position > 0 ? `Position ${queue.position}.` : "Waiting in the queue.";
  const eta = etaLabel(queue.eta_seconds);
  return eta ? `${where} About ${eta.replace("~", "")} to merge.` : where;
}
