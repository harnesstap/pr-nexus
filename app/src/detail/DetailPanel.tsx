import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Api, Explanation, ListRow, MentionsScope, PrDetail, PrOverview } from "../api";
import { errorMessage } from "../api";
import { CloseButton } from "../CloseButton";
import { AlertIcon, CheckIcon, DeclinedIcon, GitHubIcon } from "../icons";
import { isTypingTarget } from "../list/keys";
import { ago } from "../list/model";
import { SlackContextPanel } from "../list/SlackContextPanel";
import { Tooltip } from "../Tooltip";
import { AutoMerge } from "./AutoMerge";
import { Expander } from "./Expander";
import { draftView, type DraftView } from "./draft";
import { peopleGroups, PeopleRow, requiredSeats } from "./People";
import { SlackLine } from "./SlackLine";
import { StatCards, VerdictIcon } from "./StatCards";
import { mergeVerdict, TONE_TEXT } from "./status";

interface Props {
  api: Api;
  row: ListRow;
  /** Bumps after every sync so the overview refreshes. */
  version: number;
  onClose: () => void;
  onOpen: (url: string) => void;
  /** Opens the Slack request/bump preview for this PR. */
  onRequestReview?: () => void;
  onNotice: (text: string) => void;
  /**
   * The draft flag changed here: `true`/`false` optimistically, `null` to roll the
   * row back, and `"saved"` once GitHub agreed (the caller then refreshes).
   */
  onDraftChange?: (id: number, draft: boolean | null | "saved") => void;
  /** Adds the full Slack threads and mention search (§7.10) as an expander. */
  slack?: { mentions: MentionsScope };
}

/** Panel width as a share of the window, remembered across launches. */
export const WIDTH_KEY = "pr-nexus.detail-width";
export const DEFAULT_WIDTH = 0.4;
const MIN_WIDTH = 0.28;
const MAX_WIDTH = 0.7;
const MIN_PX = 380;

export function clampWidth(share: number): number {
  return Number.isFinite(share) ? Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, share)) : DEFAULT_WIDTH;
}

function storedWidth(): number {
  try {
    const raw = window.localStorage.getItem(WIDTH_KEY);
    return raw === null ? DEFAULT_WIDTH : clampWidth(Number(raw));
  } catch {
    return DEFAULT_WIDTH;
  }
}

/**
 * The PR detail slide-over (§7.2): docked on the right while the list stays usable, so a
 * click or ↑/↓ in the list switches the PR. Esc or ✕ closes it.
 */
export function DetailPanel({ api, row, version, onClose, onOpen, onRequestReview, onNotice, onDraftChange, slack }: Props) {
  const [overview, setOverview] = useState<PrOverview | null>(null);
  const [overviewError, setOverviewError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [detail, setDetail] = useState<PrDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [width, setWidth] = useState(storedWidth);
  const [flipping, setFlipping] = useState(false);
  const panelRef = useRef<HTMLElement>(null);

  // Cache first so the cards show at once, then GitHub; again after every sync.
  useEffect(() => {
    let cancel = false;
    api
      .prOverview(row.id, false)
      .then((cached) => !cancel && cached && setOverview((current) => current ?? cached))
      .catch(() => undefined);
    api
      .prOverview(row.id, true)
      .then((fresh) => {
        if (cancel) return;
        setOverviewError(null);
        if (fresh) setOverview(fresh);
      })
      .catch((err: unknown) => !cancel && setOverviewError(errorMessage(err)))
      .finally(() => !cancel && setLoaded(true));
    return () => {
      cancel = true;
    };
  }, [api, row.id, version]);

  useEffect(() => {
    let cancel = false;
    api
      .prDetail(row.id)
      .then((next) => !cancel && setDetail(next))
      .catch((err: unknown) => !cancel && setDetailError(errorMessage(err)));
    return () => {
      cancel = true;
    };
  }, [api, row.id]);

  // Esc closes the panel before the list sees it (the list clears its selection on Esc).
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key !== "Escape" || document.querySelector('[role="dialog"][aria-modal="true"]')) {
        return;
      }
      const target = event.target instanceof HTMLElement ? event.target : null;
      if (isTypingTarget(target) && !panelRef.current?.contains(target)) {
        return;
      }
      event.stopPropagation();
      onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  function resize(share: number) {
    const next = clampWidth(share);
    setWidth(next);
    try {
      window.localStorage.setItem(WIDTH_KEY, String(next));
    } catch {
      // Private mode: the width just isn't remembered.
    }
  }

  function startDrag(event: React.PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    const move = (moved: PointerEvent) => resize((window.innerWidth - moved.clientX) / window.innerWidth);
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  async function toggleDraft() {
    const view = draftView(row, overview);
    if (!view.available || flipping || !overview) return;
    const before = overview;
    const next = !view.draft;
    setFlipping(true);
    setOverview({ ...before, is_draft: next, merge_state: next ? "draft" : before.merge_state === "draft" ? "unknown" : before.merge_state });
    onDraftChange?.(row.id, next);
    try {
      const fresh = await api.setPrDraft(row.id, next);
      setOverview(fresh);
      onDraftChange?.(row.id, "saved");
      onNotice(next ? `#${row.number} is a draft again` : `#${row.number} is ready for review`);
    } catch (err) {
      setOverview(before);
      onDraftChange?.(row.id, null);
      onNotice(`Couldn't ${next ? "convert to draft" : "mark ready for review"}: ${errorMessage(err)}`);
    } finally {
      setFlipping(false);
    }
  }

  const draft = draftView(row, overview);
  const state = overview ? (overview.state === "open" && overview.is_draft ? "draft" : overview.state) : row.state === "open" && row.is_draft ? "draft" : row.state;
  const verdict = overview ? mergeVerdict(overview) : null;
  const files = detail ? `${detail.files.length}${detail.files_truncated ? "+" : ""}` : "…";

  return (
    <aside
      ref={panelRef}
      aria-labelledby="pr-detail-title"
      data-testid="detail-panel"
      data-detail-panel=""
      style={{ width: `${width * 100}%`, minWidth: MIN_PX }}
      className="relative flex h-full shrink-0 flex-col border-l border-slate-800 bg-slate-900 text-sm text-slate-200 shadow-[-12px_0_32px_rgba(0,0,0,0.35)]"
    >
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the detail panel"
        aria-valuenow={Math.round(width * 100)}
        aria-valuemin={MIN_WIDTH * 100}
        aria-valuemax={MAX_WIDTH * 100}
        tabIndex={0}
        onPointerDown={startDrag}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
            event.preventDefault();
            event.stopPropagation();
            resize(width + (event.key === "ArrowLeft" ? 0.02 : -0.02));
          }
        }}
        className="absolute inset-y-0 -left-1 z-10 w-2 cursor-col-resize outline-none hover:bg-violet-500/30 focus-visible:bg-violet-500/40"
      />
      <header className="flex items-start gap-3 border-b border-slate-800 px-5 pb-3 pt-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <StatePill state={state} draft={draft} busy={flipping} onToggle={() => void toggleDraft()} />
            <span className="truncate text-xs text-slate-500">
              {row.repo}
              {" · "}
              <span className="font-mono">{overview?.base_ref ?? "…"}</span>
              {" ← "}
              <span className="font-mono">{overview?.head_ref ?? row.head_ref}</span>
            </span>
          </div>
          <h2 id="pr-detail-title" className="mt-1.5 text-base font-semibold leading-snug">
            <span className="text-slate-500">#{row.number}</span> {row.title}
          </h2>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <IconButton label="Open on GitHub" shortcut="O" onClick={() => onOpen(row.url)}>
            <GitHubIcon size={16} />
          </IconButton>
          <CloseButton label="Close details" onClick={onClose} />
        </div>
      </header>

      <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4">
        {!overview && !loaded && <Skeleton />}
        {!overview && loaded && (
          <p className="rounded-lg border border-slate-800 p-3 text-slate-400">{overviewError ? `Could not load this pull request: ${overviewError}` : "This provider has no overview for this pull request."}</p>
        )}
        {overview && (
          <>
            <StatCards overview={overview} onOpen={onOpen} />
            {verdict && (
              <div className="flex flex-wrap items-center gap-3 rounded-lg border border-slate-800 bg-slate-950/40 p-3" data-testid="merge-verdict">
                <span className={TONE_TEXT[verdict.tone]}>
                  <VerdictIcon tone={verdict.tone} />
                </span>
                <Tooltip
                  label="How this is worked out"
                  detail="GitHub doesn't say exactly who may press Merge, so this comes from the merge state, your permission and the branch rules your token can read."
                  align="start"
                >
                  <div tabIndex={0} className="min-w-0 flex-1 outline-none">
                    <p className={`font-medium ${TONE_TEXT[verdict.tone]}`}>{verdict.title}</p>
                    <p className="text-xs text-slate-400">{verdict.reason}</p>
                  </div>
                </Tooltip>
                {overview.state === "open" && overview.is_draft && (
                  <DraftAction view={draft} busy={flipping} onClick={() => void toggleDraft()} />
                )}
                {overview.state === "open" && <AutoMerge api={api} prId={row.id} overview={overview} onChange={setOverview} onNotice={onNotice} />}
              </div>
            )}
            {/* The one primary action, right under "can I merge": asking for review is what unblocks it. */}
            <SlackLine api={api} prId={row.id} onOpen={onOpen} onRequest={onRequestReview} />
            {overviewError && <p className="text-xs text-amber-200">Showing the copy from {ago(overview.fetched_at, new Date())}; refreshing failed: {overviewError}</p>}
            <PeopleRow
              groups={peopleGroups(overview, detail ? requiredSeats(overview.reviewers, detail.mandatory_teams, detail.approved_teams) : [])}
              missing={overview.required_approvals === null ? 0 : Math.max(0, overview.required_approvals - overview.approvals)}
            />
          </>
        )}
        {!overview && <SlackLine api={api} prId={row.id} onOpen={onOpen} onRequest={onRequestReview} />}
        <div>
          <Expander title={`Files changed (${files})`} aside={detail?.codeowners_path ? "with code owners" : undefined}>
            <FilesBody detail={detail} error={detailError} />
          </Expander>
          <Expander title="Details">
            <DetailsBody detail={detail} error={detailError} />
          </Expander>
          {slack && (
            <Expander title="Slack threads and mentions">
              <SlackContextPanel api={api} prId={row.id} mentions={slack.mentions} onOpen={onOpen} />
            </Expander>
          )}
        </div>
        {overview && <p className="text-[11px] text-slate-600">Updated {ago(overview.fetched_at, new Date())}</p>}
      </div>
    </aside>
  );
}

const PILL: Record<string, { label: string; tone: string }> = {
  open: { label: "Open", tone: "bg-teal-500/15 text-teal-200 ring-teal-400/30" },
  draft: { label: "Draft", tone: "bg-slate-700/60 text-slate-300 ring-slate-500/40" },
  merged: { label: "Merged", tone: "bg-violet-500/20 text-violet-200 ring-violet-400/30" },
  closed: { label: "Closed", tone: "bg-rose-500/15 text-rose-200 ring-rose-400/30" },
};

/** The state pill doubles as the draft ↔ ready toggle while the PR is open. */
function StatePill({ state, draft, busy, onToggle }: { state: string; draft: DraftView; busy: boolean; onToggle: () => void }) {
  const pill = PILL[state] ?? PILL.open;
  const look = `shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium ring-1 ${pill.tone}`;
  if (state === "merged" || state === "closed") {
    return <span className={look}>{pill.label}</span>;
  }
  const button = (
    <button
      type="button"
      data-testid="state-pill"
      aria-label={`${pill.label}: ${draft.action.toLowerCase()}`}
      disabled={!draft.available || busy}
      onClick={onToggle}
      className={`${look} inline-flex items-center gap-1 enabled:hover:ring-2 disabled:cursor-default`}
    >
      {busy ? "Saving…" : pill.label}
      <span aria-hidden="true" className="text-[9px] opacity-60">⇄</span>
    </button>
  );
  if (!draft.available) {
    return (
      <Tooltip label={`Can't ${draft.action.toLowerCase()}`} detail={draft.reason ?? undefined} align="start">
        <span tabIndex={0} data-testid="state-pill-disabled" className="inline-flex rounded-full outline-none focus-visible:ring-2 focus-visible:ring-violet-400">
          {button}
        </span>
      </Tooltip>
    );
  }
  return (
    <Tooltip label={draft.action} detail={draft.draft ? "Reviewers get notified and it can merge" : "Hides it from merging until you mark it ready again"} align="start">
      {button}
    </Tooltip>
  );
}

function DraftAction({ view, busy, onClick }: { view: DraftView; busy: boolean; onClick: () => void }) {
  const button = (
    <button
      type="button"
      disabled={!view.available || busy}
      onClick={onClick}
      className="rounded-md border border-amber-400/50 bg-amber-500/10 px-2.5 py-1 text-xs font-medium text-amber-100 enabled:hover:bg-amber-500/20 disabled:border-slate-700 disabled:bg-slate-800 disabled:text-slate-500"
    >
      {busy ? "Saving…" : "Mark ready for review"}
    </button>
  );
  return view.available ? (
    <Tooltip label="Mark ready for review" detail="Leaves draft so reviewers are notified and it can merge" side="top" align="end">
      {button}
    </Tooltip>
  ) : (
    <Tooltip label="Can't mark ready" detail={view.reason ?? undefined} side="top" align="end">
      <span tabIndex={0} className="inline-flex rounded-md outline-none focus-visible:ring-2 focus-visible:ring-violet-400">
        {button}
      </span>
    </Tooltip>
  );
}

function IconButton({ label, detail, shortcut, onClick, children }: { label: string; detail?: string; shortcut?: string; onClick: () => void; children: ReactNode }) {
  return (
    <Tooltip label={label} detail={detail} shortcut={shortcut} align="end">
      <button type="button" aria-label={label} onClick={onClick} className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100">
        {children}
      </button>
    </Tooltip>
  );
}

function Skeleton() {
  return (
    <div aria-label="Loading pull request" role="status" className="space-y-4">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {[0, 1, 2, 3].map((item) => (
          <div key={item} className="h-[5.5rem] animate-pulse rounded-lg bg-slate-800/70" />
        ))}
      </div>
      <div className="h-14 animate-pulse rounded-lg bg-slate-800/70" />
      <div className="h-4 w-1/3 animate-pulse rounded bg-slate-800/70" />
      <div className="flex gap-2">
        {[0, 1, 2].map((item) => (
          <div key={item} className="h-7 w-28 animate-pulse rounded-full bg-slate-800/70" />
        ))}
      </div>
    </div>
  );
}

function Pending({ error }: { error: string | null }) {
  return error ? <p className="text-xs text-rose-300">{error}</p> : <p className="text-xs text-slate-500">Reading code owners…</p>;
}

function FilesBody({ detail, error }: { detail: PrDetail | null; error: string | null }) {
  if (!detail) {
    return <Pending error={error} />;
  }
  if (detail.files.length === 0) {
    return <p className="text-xs text-slate-500">No changed files.</p>;
  }
  return (
    <>
      <ul className="max-h-72 overflow-auto rounded-lg border border-slate-800" aria-label="Changed files">
        {detail.files.map((file) => (
          <li key={file.path} className="flex items-center justify-between gap-3 border-t border-slate-800 px-3 py-1.5 first:border-t-0">
            <span className="truncate font-mono text-xs text-slate-300">{file.path}</span>
            <span className="flex shrink-0 gap-1">
              {file.teams.length === 0 ? (
                <span className="text-xs text-slate-600">no owner</span>
              ) : (
                file.teams.map((team) => (
                  <span key={team} className="rounded bg-indigo-500/15 px-1.5 text-[11px] text-indigo-200">
                    {team}
                  </span>
                ))
              )}
            </span>
          </li>
        ))}
      </ul>
      {detail.files_truncated && <p className="mt-1 text-xs text-slate-500">GitHub listed only the first files.</p>}
    </>
  );
}

const TONE_ICON: Record<Explanation["tone"], { icon: ReactNode; tone: string }> = {
  good: { icon: <CheckIcon size={14} />, tone: "text-teal-300" },
  warn: { icon: <AlertIcon size={14} />, tone: "text-amber-300" },
  bad: { icon: <DeclinedIcon size={14} />, tone: "text-rose-300" },
  info: { icon: <span className="block h-1.5 w-1.5 rounded-full bg-slate-500" />, tone: "text-slate-500" },
};

function DetailsBody({ detail, error }: { detail: PrDetail | null; error: string | null }) {
  if (!detail) {
    return <Pending error={error} />;
  }
  const lines: Explanation[] = [...detail.explanations, { tone: "info", text: detail.protection.replace(/\.?$/, ".") }];
  return (
    <ul className="space-y-1.5" aria-label="Details">
      {lines.map((line) => (
        <li key={line.text} className="flex items-start gap-2 text-xs text-slate-300">
          <span className={`mt-0.5 flex h-3.5 w-3.5 shrink-0 items-center justify-center ${TONE_ICON[line.tone].tone}`}>{TONE_ICON[line.tone].icon}</span>
          <span>{line.text}</span>
        </li>
      ))}
    </ul>
  );
}
