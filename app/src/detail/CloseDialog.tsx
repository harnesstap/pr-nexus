import { useState, type KeyboardEvent } from "react";
import type { Api, CloseReport } from "../api";
import { errorMessage } from "../api";
import { plainText } from "../textInput";

/** A pull request the dialog can name before GitHub answers. */
export interface CloseTarget {
  id: number;
  repo: string;
  number: number;
  title: string;
}

interface Props {
  api: Api;
  ids: number[];
  known?: CloseTarget[];
  onDismiss: () => void;
  /** Closed and skipped pull requests. The caller drops them from the open list. */
  onClosed: (ids: number[]) => void;
  onNotice: (text: string) => void;
}

/** The confirm button. One pull request stays "Close PR"; several name the count. */
export function closeButtonLabel(count: number): string {
  return count === 1 ? "Close PR" : `Close ${count} PRs`;
}

export function closeDialogTitle(count: number): string {
  return count === 1 ? "Close this pull request?" : `Close ${count} pull requests?`;
}

function targetLabel(target: CloseTarget | undefined, id: number): string {
  if (!target || target.number <= 0) {
    return `Pull request ${id}`;
  }
  const where = target.repo ? `${target.repo}#${target.number}` : `#${target.number}`;
  return target.title ? `${where} ${target.title}` : where;
}

function reportLabel(report: CloseReport): string {
  if (report.number > 0) {
    const where = report.repo ? `${report.repo}#${report.number}` : `#${report.number}`;
    return report.title ? `${where} ${report.title}` : where;
  }
  return `Pull request ${report.id}`;
}

function noticeFor(reports: CloseReport[]): string {
  if (reports.length === 1) {
    const only = reports[0];
    if (only.outcome === "closed") {
      return `Closed ${reportLabel(only)}.`;
    }
    if (only.outcome === "skipped") {
      return only.message ?? "Already closed.";
    }
  }
  const closed = reports.filter((report) => report.outcome === "closed").length;
  const skipped = reports.filter((report) => report.outcome === "skipped").length;
  const parts: string[] = [];
  if (closed > 0) {
    parts.push(`Closed ${closed} pull request${closed === 1 ? "" : "s"}.`);
  }
  if (skipped > 0) {
    parts.push(`${skipped} already closed or merged.`);
  }
  return parts.join(" ");
}

/**
 * Confirms a close. Enter never confirms, so a newline in the comment or a
 * focused button cannot close the pull request by accident. The branch is
 * left in place and nothing is merged.
 */
export function CloseDialog({ api, ids, known = [], onDismiss, onClosed, onNotice }: Props) {
  const [comment, setComment] = useState("");
  const [pending, setPending] = useState(ids);
  const [busy, setBusy] = useState(false);
  const [failures, setFailures] = useState<CloseReport[]>([]);
  const [error, setError] = useState<string | null>(null);
  const knownById = new Map(known.map((target) => [target.id, target]));
  const title = closeDialogTitle(pending.length);

  async function confirm() {
    if (busy || pending.length === 0) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const reports = await api.closePullRequests(pending, comment.trim());
      const done = reports
        .filter((report) => report.outcome === "closed" || report.outcome === "skipped")
        .map((report) => report.id);
      const failed = reports.filter((report) => report.outcome === "failed");
      if (done.length > 0) {
        onClosed(done);
      }
      if (failed.length === 0) {
        const text = noticeFor(reports);
        if (text) {
          onNotice(text);
        }
        onDismiss();
        return;
      }
      setPending(failed.map((report) => report.id));
      setFailures(failed);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (!busy) {
        onDismiss();
      }
      return;
    }
    if (event.key === "Enter" && !(event.target instanceof HTMLTextAreaElement)) {
      event.preventDefault();
      event.stopPropagation();
    }
  }

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center overflow-auto bg-black/50 p-8"
      onMouseDown={(event) => {
        if (!busy && event.target === event.currentTarget) {
          onDismiss();
        }
      }}
      onKeyDownCapture={onKeyDown}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="close-pr-title"
        className="w-full max-w-lg rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <h2 id="close-pr-title" className="text-base font-semibold">
          {title}
        </h2>
        <ul className="mt-3 max-h-40 space-y-1 overflow-y-auto text-slate-300">
          {pending.map((id) => (
            <li key={id} className="truncate">
              {targetLabel(knownById.get(id), id)}
            </li>
          ))}
        </ul>
        <p className="mt-3 text-slate-400">
          {pending.length === 1
            ? "If you write a comment, it is posted before the pull request closes. Leave it empty to close without a comment."
            : "If you write a comment, the same comment is posted on each pull request before it closes. Leave it empty to close without a comment."}
        </p>
        <p className="mt-1 text-slate-400">The branch stays. Nothing is merged.</p>
        <label className="mt-4 block text-slate-300" htmlFor="close-pr-comment">
          Comment (optional)
        </label>
        <textarea
          {...plainText}
          id="close-pr-comment"
          aria-label="Comment (optional)"
          rows={4}
          autoFocus
          value={comment}
          disabled={busy}
          onChange={(event) => setComment(event.target.value)}
          className="mt-1 w-full resize-y rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none focus:border-slate-500"
        />
        {failures.length > 0 && (
          <div role="alert" className="mt-3 space-y-1 rounded-md border border-rose-800/80 bg-rose-950/40 p-3 text-rose-100">
            <p>Some pull requests could not be closed.</p>
            <ul className="space-y-1">
              {failures.map((report) => (
                <li key={report.id}>
                  {reportLabel(report)}: {report.message ?? "Could not close."}
                </li>
              ))}
            </ul>
          </div>
        )}
        {error && (
          <p role="alert" className="mt-3 text-rose-200">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            className="rounded-md border border-slate-700 px-3 py-1.5 text-slate-200 hover:bg-slate-800 disabled:text-slate-500"
            disabled={busy}
            onClick={onDismiss}
          >
            Cancel
          </button>
          <button
            type="button"
            className="rounded-md bg-rose-600 px-3 py-1.5 font-medium text-white hover:bg-rose-500 disabled:bg-rose-900"
            disabled={busy || pending.length === 0}
            onClick={() => void confirm()}
          >
            {busy ? "Closing…" : closeButtonLabel(pending.length)}
          </button>
        </div>
      </div>
    </div>
  );
}
