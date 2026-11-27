import { useEffect, useState, type KeyboardEvent, type ReactNode } from "react";
import type { ApprovalPreview, ApproveReport, Api } from "../api";
import { errorMessage } from "../api";
import { CheckIcon } from "../icons";
import { plainText } from "../textInput";
import { Tooltip } from "../Tooltip";

/** A selected pull request the dialog can name before GitHub answers. */
export interface ApproveTargetRow {
  id: number;
  repo: string;
  number: number;
  title: string;
}

interface Props {
  api: Api;
  rows: ApproveTargetRow[];
  onDismiss: () => void;
  /** Approvals went out; re-read the list. */
  onApproved: (ids: number[]) => void;
  onNotice: (text: string) => void;
}

type Phase =
  | { kind: "checking"; ids: number[] }
  | { kind: "ready"; rows: ApprovalPreview[] }
  | { kind: "sending"; rows: ApprovalPreview[] }
  | { kind: "done"; reports: ApproveReport[] };

export function approveButtonLabel(count: number): string {
  if (count === 0) {
    return "Approve";
  }
  return count === 1 ? "Approve PR" : `Approve ${count} PRs`;
}

export function approveDialogTitle(count: number): string {
  if (count === 0) {
    return "Nothing to approve";
  }
  return count === 1 ? "Approve 1 pull request?" : `Approve ${count} pull requests?`;
}

function where(row: { repo: string; number: number; id: number }): string {
  if (row.number <= 0) {
    return `Pull request ${row.id}`;
  }
  return row.repo ? `${row.repo}#${row.number}` : `#${row.number}`;
}

export function approvedNotice(count: number): string {
  return count === 1 ? "Approved 1 pull request." : `Approved ${count} pull requests.`;
}

/** "Approved 3 · Skipped 1 · Failed 2", leaving out empty parts. */
export function resultSummary(reports: ApproveReport[]): string {
  const count = (outcome: ApproveReport["outcome"]) => reports.filter((report) => report.outcome === outcome).length;
  return [
    ["Approved", count("approved")],
    ["Skipped", count("skipped")],
    ["Failed", count("failed")],
  ]
    .filter(([, n]) => Number(n) > 0)
    .map(([label, n]) => `${label} ${n}`)
    .join(" · ");
}

function DraftBadge() {
  return <span className="shrink-0 rounded border border-slate-600 px-1 text-xs text-slate-400">Draft</span>;
}

function Row({ label, title, children }: { label: string; title: string; children?: ReactNode }) {
  const text = title ? `${label} ${title}` : label;
  return (
    <li className="min-w-0">
      <div className="flex min-w-0 items-center gap-2">
        <Tooltip label={text} wide align="start" className="min-w-0 max-w-full">
          <span className="block truncate">
            <span className="text-slate-400">{label}</span> {title}
          </span>
        </Tooltip>
        {children}
      </div>
    </li>
  );
}

/**
 * Confirms a batch approval (§8.2 Approve pull requests). It reads each pull request fresh,
 * lists what will be approved and why the rest are skipped, then approves
 * exactly the head commits it showed. Enter never confirms.
 */
export function ApproveDialog({ api, rows, onDismiss, onApproved, onNotice }: Props) {
  const [phase, setPhase] = useState<Phase>({ kind: "checking", ids: rows.map((row) => row.id) });
  const [comment, setComment] = useState("");
  const [error, setError] = useState<string | null>(null);
  const known = new Map(rows.map((row) => [row.id, row]));
  const checkingKey = phase.kind === "checking" ? phase.ids.join(",") : null;

  useEffect(() => {
    if (checkingKey === null) {
      return;
    }
    let live = true;
    const ids = checkingKey.split(",").map(Number);
    api
      .previewApprovals(ids)
      .then((preview) => {
        if (live) {
          setPhase({ kind: "ready", rows: preview });
        }
      })
      .catch((err: unknown) => {
        if (live) {
          setError(errorMessage(err));
          setPhase({ kind: "ready", rows: [] });
        }
      });
    return () => {
      live = false;
    };
  }, [api, checkingKey]);

  const busy = phase.kind === "sending" || phase.kind === "checking";
  const preview = phase.kind === "ready" || phase.kind === "sending" ? phase.rows : [];
  const approving = preview.filter((row) => row.action === "approve");
  const skipping = preview.filter((row) => row.action === "skip");

  async function confirm() {
    if (phase.kind !== "ready" || approving.length === 0) {
      return;
    }
    setPhase({ kind: "sending", rows: phase.rows });
    setError(null);
    try {
      const reports = await api.approvePullRequests(
        approving.map((row) => ({ id: row.id, head_sha: row.head_sha })),
        comment.trim(),
      );
      const approved = reports.filter((report) => report.outcome === "approved").map((report) => report.id);
      if (approved.length > 0) {
        onApproved(approved);
      }
      if (approved.length === reports.length && skipping.length === 0) {
        onNotice(approvedNotice(approved.length));
        onDismiss();
        return;
      }
      const skippedEarlier: ApproveReport[] = skipping.map((row) => ({
        id: row.id,
        repo: row.repo,
        number: row.number,
        title: row.title,
        outcome: "skipped",
        reason: row.reason,
        message: row.message,
        draft: row.draft,
      }));
      setPhase({ kind: "done", reports: [...reports, ...skippedEarlier] });
    } catch (err) {
      setError(errorMessage(err));
      setPhase({ kind: "ready", rows: phase.rows });
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (phase.kind !== "sending") {
        onDismiss();
      }
      return;
    }
    if (event.key === "Enter" && !(event.target instanceof HTMLTextAreaElement)) {
      event.preventDefault();
      event.stopPropagation();
    }
  }

  const title =
    phase.kind === "done"
      ? "Approval results"
      : phase.kind === "checking"
        ? "Checking pull requests…"
        : approveDialogTitle(approving.length);
  const failed = phase.kind === "done" ? phase.reports.filter((report) => report.outcome === "failed") : [];

  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center overflow-auto bg-black/50 p-8"
      onMouseDown={(event) => {
        if (phase.kind !== "sending" && event.target === event.currentTarget) {
          onDismiss();
        }
      }}
      onKeyDownCapture={onKeyDown}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="approve-prs-title"
        className="w-full max-w-lg rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <h2 id="approve-prs-title" className="text-base font-semibold">
          {title}
        </h2>
        {phase.kind === "checking" && (
          <ul aria-busy="true" className="mt-3 max-h-60 space-y-1 overflow-y-auto text-slate-400">
            {phase.ids.map((id) => {
              const row = known.get(id);
              return <Row key={id} label={row ? where(row) : `Pull request ${id}`} title={row?.title ?? ""} />;
            })}
          </ul>
        )}
        {(phase.kind === "ready" || phase.kind === "sending") && (
          <>
            {approving.length > 0 && (
              <section aria-label="Will approve" className="mt-3">
                <h3 className="text-xs font-medium tracking-wide text-slate-400 uppercase">Will Approve ({approving.length})</h3>
                <ul className="mt-1 max-h-48 space-y-1 overflow-y-auto text-slate-200">
                  {approving.map((row) => (
                    <Row key={row.id} label={where(row)} title={row.title}>
                      {row.draft && <DraftBadge />}
                      {row.approved_earlier && <span className="shrink-0 text-xs text-slate-500">Re-approve</span>}
                    </Row>
                  ))}
                </ul>
              </section>
            )}
            {skipping.length > 0 && (
              <section aria-label="Skipped" className="mt-3">
                <h3 className="text-xs font-medium tracking-wide text-slate-400 uppercase">Skipped ({skipping.length})</h3>
                <ul className="mt-1 max-h-40 space-y-1 overflow-y-auto text-slate-400">
                  {skipping.map((row) => (
                    <li key={row.id} className="min-w-0">
                      <span className="block truncate">
                        {where(row)} <span className="text-slate-500">· {row.message ?? "Skipped."}</span>
                      </span>
                    </li>
                  ))}
                </ul>
              </section>
            )}
            {approving.some((row) => row.draft) && (
              <p className="mt-3 text-slate-400">Drafts can be approved, but they can't be merged until marked ready.</p>
            )}
            {approving.length > 0 && (
              <>
                <p className="mt-3 text-slate-400">
                  Each approval is for the commit checked just now. If someone pushes before you confirm, that PR is not approved. Nothing is
                  merged.
                </p>
                <label className="mt-4 block text-slate-300" htmlFor="approve-prs-comment">
                  Comment (optional)
                </label>
                <textarea
                  {...plainText}
                  id="approve-prs-comment"
                  aria-label="Comment (optional)"
                  rows={3}
                  value={comment}
                  disabled={phase.kind === "sending"}
                  onChange={(event) => setComment(event.target.value)}
                  className="mt-1 w-full resize-y rounded-md border border-slate-700 bg-slate-950 px-3 py-2 text-slate-100 outline-none focus:border-slate-500"
                />
              </>
            )}
          </>
        )}
        {phase.kind === "done" && (
          <>
            <p className="mt-1 text-slate-400">{resultSummary(phase.reports)}</p>
            <ul aria-label="Results" className="mt-3 max-h-72 space-y-1.5 overflow-y-auto">
              {phase.reports.map((report) => (
                <li key={report.id} className="flex min-w-0 items-start gap-2">
                  <span
                    className={`mt-0.5 w-16 shrink-0 text-xs font-medium ${
                      report.outcome === "approved"
                        ? "text-emerald-300"
                        : report.outcome === "failed"
                          ? "text-rose-300"
                          : "text-slate-500"
                    }`}
                  >
                    {report.outcome === "approved" ? "Approved" : report.outcome === "failed" ? "Failed" : "Skipped"}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-slate-200">
                      {where(report)} {report.title}
                    </span>
                    {report.message && (
                      <span className={`block text-xs ${report.outcome === "failed" ? "text-rose-200" : "text-slate-500"}`}>
                        {report.message}
                      </span>
                    )}
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
        {error && (
          <p role="alert" className="mt-3 text-rose-200">
            {error}
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          {phase.kind === "done" ? (
            <>
              {failed.length > 0 && (
                <button
                  type="button"
                  className="rounded-md border border-slate-700 px-3 py-1.5 text-slate-200 hover:bg-slate-800"
                  onClick={() => {
                    setError(null);
                    setPhase({ kind: "checking", ids: failed.map((report) => report.id) });
                  }}
                >
                  Check failed again
                </button>
              )}
              <button
                type="button"
                className="rounded-md bg-slate-700 px-3 py-1.5 font-medium text-white hover:bg-slate-600"
                onClick={onDismiss}
              >
                Done
              </button>
            </>
          ) : (
            <>
              <button
                type="button"
                className="rounded-md border border-slate-700 px-3 py-1.5 text-slate-200 hover:bg-slate-800 disabled:text-slate-500"
                disabled={phase.kind === "sending"}
                onClick={onDismiss}
              >
                Cancel
              </button>
              <button
                type="button"
                className="flex items-center gap-1.5 rounded-md bg-emerald-600 px-3 py-1.5 font-medium text-white hover:bg-emerald-500 disabled:bg-emerald-900 disabled:text-slate-300"
                disabled={busy || approving.length === 0}
                onClick={() => void confirm()}
              >
                <CheckIcon />
                {phase.kind === "sending" ? "Approving…" : approveButtonLabel(approving.length)}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
