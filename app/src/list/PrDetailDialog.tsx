import { useEffect, useRef, useState } from "react";
import type { Api, ListRow, MentionsScope, PrDetail } from "../api";
import { errorMessage } from "../api";
import { SlackContextPanel } from "./SlackContextPanel";

interface Props {
  api: Api;
  row: ListRow;
  onClose: () => void;
  onOpen: (url: string) => void;
  /** Opens the Slack request/bump preview for this PR. */
  onRequestReview?: () => void;
  /** Shows the Slack panel (§7.10). Off in the wizard and in tests that do not need it. */
  slack?: { mentions: MentionsScope };
}

const READINESS_LABEL: Record<PrDetail["readiness"], string> = {
  ready: "Ready for review",
  blocked: "Blocked",
  unknown: "Unknown",
};

const READINESS_TONE: Record<PrDetail["readiness"], string> = {
  ready: "bg-emerald-500/20 text-emerald-200",
  blocked: "bg-rose-500/20 text-rose-200",
  unknown: "bg-amber-500/20 text-amber-200",
};

/** One PR's readiness and code owners, with the reasons (§6, §7.2). */
export function PrDetailDialog({ api, row, onClose, onOpen, onRequestReview, slack }: Props) {
  const [detail, setDetail] = useState<PrDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    // ListScreen keys this dialog by PR id, so state starts empty for each PR.
    let cancel = false;
    api
      .prDetail(row.id)
      .then((next) => !cancel && setDetail(next))
      .catch((err: unknown) => !cancel && setError(errorMessage(err)));
    closeRef.current?.focus();
    return () => {
      cancel = true;
    };
  }, [api, row.id]);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.stopPropagation();
        onClose();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-30 flex items-start justify-center overflow-auto bg-black/50 p-8" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="pr-detail-title"
        className={`w-full ${slack ? "max-w-6xl" : "max-w-3xl"} rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl`}
      >
        <header className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <h2 id="pr-detail-title" className="truncate text-base font-semibold">
              <span className="text-slate-500">#{row.number}</span> {row.title}
            </h2>
            <p className="text-xs text-slate-500">
              {row.repo}
              {detail && ` · ${detail.base_ref} ← ${detail.head_sha.slice(0, 7)}`}
            </p>
          </div>
          <div className="flex shrink-0 gap-2">
            {onRequestReview && (
              <button type="button" className="rounded px-2 py-1 text-violet-300 hover:bg-slate-800" onClick={onRequestReview}>
                {row.slack ? "Bump on Slack" : "Request on Slack"}
              </button>
            )}
            <button type="button" className="rounded px-2 py-1 text-slate-400 hover:bg-slate-800" onClick={() => onOpen(row.url)}>
              Open on GitHub
            </button>
            <button ref={closeRef} type="button" className="rounded px-2 py-1 text-slate-400 hover:bg-slate-800" onClick={onClose}>
              Close
            </button>
          </div>
        </header>

        <div className={slack ? "mt-4 grid gap-5 lg:grid-cols-[minmax(0,1fr)_24rem]" : ""}>
          <div className="min-w-0">
            {error && <p className="mt-4 text-rose-300">{error}</p>}
            {!detail && !error && <p className="mt-4 text-slate-400">Checking code owners…</p>}
            {detail && <Body detail={detail} />}
          </div>
          {slack && <SlackContextPanel api={api} prId={row.id} mentions={slack.mentions} onOpen={onOpen} />}
        </div>
      </div>
    </div>
  );
}

function Body({ detail }: { detail: PrDetail }) {
  return (
    <>
      <section aria-label="Readiness" className="mt-4">
        <span className={`rounded px-2 py-0.5 text-xs font-medium ${READINESS_TONE[detail.readiness]}`}>{READINESS_LABEL[detail.readiness]}</span>
        {detail.readiness_notes.length > 0 && (
          <ul className="mt-2 list-disc pl-5 text-slate-300">
            {detail.readiness_notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        )}
      </section>

      <section aria-labelledby="owners-heading" className="mt-5">
        <h3 id="owners-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
          Code owners
        </h3>
        <p className="mt-1 text-slate-300">
          {detail.codeowners_path ?? "No CODEOWNERS file"}
          {" · "}
          {requirementLabel(detail)}
        </p>
        {detail.codeowners_error && <p className="mt-1 text-amber-200">{detail.codeowners_error}</p>}
        <dl className="mt-2 grid grid-cols-[10rem_1fr] gap-x-3 gap-y-1">
          <Teams label="Must approve" teams={detail.mandatory_teams} />
          <Teams label="Approved" teams={detail.approved_teams} />
          <Teams label="Optional" teams={detail.optional_teams} />
          <Teams label="Requested on GitHub" teams={detail.requested_teams} />
          <Teams label="Users (not posted)" teams={detail.users_ignored} />
        </dl>
        {detail.attribution_unknown.length > 0 && (
          <div className="mt-2 rounded border border-amber-500/30 bg-amber-500/5 p-2">
            <p className="text-amber-200">Approvals that could not be matched to a team</p>
            <ul className="mt-1 list-disc pl-5 text-slate-300">
              {detail.attribution_unknown.map((gap) => (
                <li key={gap.team}>
                  {gap.team}: {gap.reasons.join("; ")}
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      <section aria-labelledby="why-heading" className="mt-5">
        <h3 id="why-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
          Why
        </h3>
        <ul className="mt-1 list-disc pl-5 text-slate-300">
          <li>{detail.codeowners_required_reason}</li>
          <li>{detail.protection}</li>
          {detail.rules_error && <li className="text-amber-200">{detail.rules_error}</li>}
          {detail.rationale.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="files-heading" className="mt-5">
        <h3 id="files-heading" className="text-xs font-medium uppercase tracking-wide text-slate-500">
          Files ({detail.files.length}
          {detail.files_truncated ? "+, list truncated" : ""})
        </h3>
        <ul className="mt-1 max-h-64 overflow-auto rounded border border-slate-800">
          {detail.files.map((file) => (
            <li key={file.path} className="flex justify-between gap-3 border-t border-slate-800 px-2 py-1 first:border-t-0">
              <span className="truncate font-mono text-xs">{file.path}</span>
              <span className="shrink-0 text-xs text-slate-400">{file.teams.length > 0 ? file.teams.join(", ") : "no owner"}</span>
            </li>
          ))}
        </ul>
        {detail.from_cache && <p className="mt-1 text-xs text-slate-500">Files and branch rules from cache (same head, base, and CODEOWNERS).</p>}
      </section>
    </>
  );
}

function Teams({ label, teams }: { label: string; teams: string[] }) {
  if (teams.length === 0) {
    return null;
  }
  return (
    <>
      <dt className="text-slate-500">{label}</dt>
      <dd>{teams.join(", ")}</dd>
    </>
  );
}

export function requirementLabel(detail: Pick<PrDetail, "codeowners_required" | "code_owner_review_pending">): string {
  if (detail.codeowners_required === "false") {
    return "code-owner review not required";
  }
  if (detail.codeowners_required === "unknown") {
    return "code-owner requirement unknown";
  }
  if (detail.code_owner_review_pending === true) {
    return "code-owner review required, still pending";
  }
  if (detail.code_owner_review_pending === false) {
    return "code-owner review required, satisfied";
  }
  return "code-owner review required, status unknown";
}
