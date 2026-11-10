import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Api, ListRow, QueueReport } from "../api";
import { errorMessage } from "../api";
import { enqueuePlan } from "./mergeQueue";

interface Props {
  api: Api;
  rows: ListRow[];
  onDismiss: () => void;
  onDone: () => void;
  onNotice: (text: string) => void;
}

function label(row: ListRow): string {
  return `${row.repo}#${row.number} ${row.title}`;
}

function noticeFor(reports: QueueReport[], fallback: string): string {
  const added = reports.filter((report) => report.outcome === "added").length;
  const skipped = reports.filter((report) => report.outcome === "skipped");
  const failed = reports.filter((report) => report.outcome === "failed");
  const parts: string[] = [];
  if (added > 0) {
    parts.push(`Added ${added} pull request${added === 1 ? "" : "s"} to the merge queue.`);
  }
  if (skipped.length > 0) {
    const first = skipped[0];
    const why = first?.message ? `: ${first.message}` : "";
    parts.push(
      skipped.length === 1
        ? `Skipped ${first ? `${first.repo}#${first.number}` : "1"}${why}.`
        : `Skipped ${skipped.length}.`,
    );
  }
  if (failed.length > 0) {
    const first = failed[0];
    parts.push(first?.message ? `Could not add ${first.repo}#${first.number}: ${first.message}` : `Could not add ${failed.length}.`);
  }
  return parts.join(" ") || fallback;
}

/**
 * Confirms a bulk add to the merge queue. Enter never confirms.
 * Pull requests that cannot join are listed and left alone.
 */
export function MergeQueueDialog({ api, rows, onDismiss, onDone, onNotice }: Props) {
  const plan = enqueuePlan(rows);
  const [busy, setBusy] = useState(false);
  const count = plan.add.length;
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    dialogRef.current?.focus();
  }, []);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (!busy) {
        onDismiss();
      }
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
    }
  }

  function confirm() {
    if (count === 0 || busy) {
      return;
    }
    setBusy(true);
    api
      .enqueuePullRequests(plan.add.map((row) => row.id))
      .then((reports) => {
        onNotice(noticeFor(reports, "Nothing was added to the merge queue."));
        onDone();
      })
      .catch((err: unknown) => {
        onNotice(`Could not add to the merge queue: ${errorMessage(err)}`);
        setBusy(false);
      });
  }

  return (
    <div
      className="fixed inset-0 z-40 flex items-center justify-center bg-slate-950/70 p-4"
      onKeyDownCapture={onKeyDown}
      onMouseDown={(event) => {
        if (!busy && event.target === event.currentTarget) {
          onDismiss();
        }
      }}
    >
      <div
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby="merge-queue-title"
        tabIndex={-1}
        className="max-h-[80vh] w-full max-w-lg overflow-y-auto rounded-lg border border-slate-700 bg-slate-900 p-4 text-sm text-slate-200 shadow-xl outline-none"
      >
        <h2 id="merge-queue-title" className="text-base font-medium text-slate-100">
          Add to the merge queue?
        </h2>
        <p className="mt-1 text-slate-400">GitHub merges them from the queue. Nothing is merged until you confirm.</p>
        {plan.add.length > 0 && (
          <ul className="mt-3 space-y-1">
            {plan.add.map((row) => (
              <li key={row.id} className="truncate">
                {label(row)}
              </li>
            ))}
          </ul>
        )}
        {plan.skip.length > 0 && (
          <ul className="mt-3 space-y-1 text-slate-400">
            {plan.skip.map(({ row, reason }) => (
              <li key={row.id} className="truncate">
                Skip {label(row)} — {reason}
              </li>
            ))}
          </ul>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            disabled={busy}
            onClick={onDismiss}
            className="rounded-md border border-slate-700 px-3 py-1 text-slate-200 hover:bg-slate-800"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={busy || count === 0}
            onClick={confirm}
            className="rounded-md bg-teal-600 px-3 py-1 font-semibold text-white enabled:hover:bg-teal-500 disabled:opacity-50"
          >
            {busy ? "Adding…" : count === 1 ? "Add 1 to the queue" : `Add ${count} to the queue`}
          </button>
        </div>
      </div>
    </div>
  );
}
