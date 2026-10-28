import { useEffect, useRef, useState } from "react";
import type { Api, BranchState, BranchStatus, PrOverview, UpdateMethod } from "../api";
import { errorMessage } from "../api";
import { CloseButton } from "../CloseButton";
import { AlertIcon, CheckIcon, ChevronDownIcon, DeclinedIcon, ExternalIcon, GitBranchIcon } from "../icons";
import { BAR_SLOT, CAPTION, CARD, VALUE } from "./StatCards";
import { TONE_TEXT, type Tone } from "./status";

/** Re-polls while GitHub is still computing mergeability, backing off each time. */
export const CHECKING_DELAYS_MS = [2000, 4000, 8000, 16000];

export interface BranchController {
  /** The card's state: the server's once it answers, a guess from the overview before. */
  view: BranchView;
  updating: UpdateMethod | null;
  error: string | null;
  confirmingRebase: boolean;
  choose: (method: UpdateMethod) => void;
  confirmRebase: () => void;
  cancelRebase: () => void;
  dismissError: () => void;
}

export interface BranchView {
  state: BranchState | "merged" | "closed" | "loading";
  base: string;
  behindBy: number | null;
  otherAuthors: string[];
  conflictsUrl: string;
}

interface Local {
  prId: number;
  status: BranchStatus | null;
  updating: UpdateMethod | null;
  error: string | null;
  confirmingRebase: boolean;
  attempts: number;
}

const blank = (prId: number): Local => ({ prId, status: null, updating: null, error: null, confirmingRebase: false, attempts: 0 });

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;

/**
 * Loads the Branch card lazily (only once the panel shows a fresh overview), polls
 * while GitHub is still checking, and runs Update branch.
 */
export function useBranch({
  api,
  prId,
  overview,
  fresh,
  onOverview,
  onNotice,
}: {
  api: Api;
  prId: number;
  overview: PrOverview | null;
  fresh: boolean;
  onOverview: (next: PrOverview) => void;
  onNotice: (text: string) => void;
}): BranchController {
  // Everything below belongs to one PR; a different id starts from scratch.
  const [local, setLocal] = useState<Local>(() => blank(prId));
  const mine = local.prId === prId ? local : blank(prId);
  const patch = (next: Partial<Local>) => setLocal((current) => ({ ...(current.prId === prId ? current : blank(prId)), ...next }));
  const { status, updating, error, confirmingRebase, attempts } = mine;
  const open = overview?.state === "open";
  const stamp = overview ? `${overview.fetched_at}|${overview.head_sha ?? ""}|${overview.mergeable}|${overview.merge_state}` : "";

  useEffect(() => {
    if (!fresh || !open) {
      return;
    }
    let live = true;
    api
      .prBranchStatus(prId, false)
      .then((next) => live && setLocal((current) => (current.prId === prId ? { ...current, status: next } : { ...blank(prId), status: next })))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [api, prId, fresh, open, stamp]);

  const latest = useRef(onOverview);
  useEffect(() => {
    latest.current = onOverview;
  });

  const view = branchView(overview, status);
  const checking = fresh && view.state === "checking";
  useEffect(() => {
    if (!checking || attempts >= CHECKING_DELAYS_MS.length) {
      return;
    }
    let live = true;
    const timer = window.setTimeout(() => {
      api
        .prOverview(prId, true)
        .then((next) => {
          if (live && next) {
            latest.current(next);
          }
        })
        .catch(() => undefined)
        .finally(() => live && setLocal((current) => (current.prId === prId ? { ...current, attempts: current.attempts + 1 } : current)));
    }, CHECKING_DELAYS_MS[attempts]);
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [api, prId, checking, attempts]);

  function run(method: UpdateMethod) {
    patch({ confirmingRebase: false, updating: method, error: null });
    api
      .updatePrBranch(prId, method)
      .then((result) => {
        onOverview(result.overview);
        patch({ status: result.branch, attempts: 0 });
        onNotice(method === "rebase" ? `Rebased onto ${view.base}.` : `Merged ${view.base} into the branch.`);
      })
      .catch((err: unknown) => patch({ error: `Couldn't update the branch: ${errorMessage(err)}` }))
      .finally(() => patch({ updating: null }));
  }

  return {
    view,
    updating,
    error,
    confirmingRebase,
    choose: (method) => (method === "rebase" ? patch({ confirmingRebase: true }) : run(method)),
    confirmRebase: () => run("rebase"),
    cancelRebase: () => patch({ confirmingRebase: false }),
    dismissError: () => patch({ error: null }),
  };
}

/** What to show; mirrors `pr_nexus_core::branch::derive_state` until the server answers. */
export function branchView(overview: PrOverview | null, status: BranchStatus | null): BranchView {
  const base = status?.base_ref ?? overview?.base_ref ?? "base";
  const shared = { base, behindBy: status?.behind_by ?? null, otherAuthors: status?.other_authors ?? [], conflictsUrl: status?.conflicts_url ?? "" };
  if (!overview) {
    return { ...shared, state: "loading" };
  }
  if (overview.state === "merged" || overview.state === "closed") {
    return { ...shared, state: overview.state };
  }
  if (overview.mergeable === "conflicting" || overview.merge_state === "dirty") {
    return { ...shared, state: "conflicts" };
  }
  if (overview.merge_state === "behind") {
    return { ...shared, state: "update_required" };
  }
  if (overview.mergeable !== "mergeable") {
    return { ...shared, state: "checking" };
  }
  return { ...shared, state: status ? status.state : "loading" };
}

/** The value, the line under it, and its tone, for each state. */
export function branchText(view: BranchView): { value: string; caption: string; tone: Tone } {
  const behind = view.behindBy === null ? `Behind ${view.base}` : `${plural(view.behindBy, "commit")} behind ${view.base}`;
  switch (view.state) {
    case "up_to_date":
      return { value: "Up to date", caption: `Up to date with ${view.base}`, tone: "good" };
    case "behind":
      return { value: "Behind", caption: behind, tone: "good" };
    case "update_required":
      return {
        value: "Update required",
        caption: `${view.behindBy === null ? "Behind" : plural(view.behindBy, "commit") + " behind"} · update required`,
        tone: "warn",
      };
    case "conflicts":
      return { value: "Conflicts", caption: `Conflicts with ${view.base}`, tone: "bad" };
    case "checking":
    case "loading":
      return { value: "Checking…", caption: `Comparing with ${view.base}`, tone: "muted" };
    case "merged":
      return { value: "Merged", caption: `Already in ${view.base}`, tone: "good" };
    case "closed":
      return { value: "Closed", caption: "Not merged", tone: "muted" };
    default: {
      const never: never = view.state;
      return never;
    }
  }
}

function StateIcon({ state }: { state: BranchView["state"] }) {
  switch (state) {
    case "up_to_date":
    case "merged":
      return <CheckIcon size={16} data-icon="check" />;
    case "behind":
      return (
        <span data-icon="check-ring" className="inline-flex rounded-full p-0.5 text-teal-300 ring-2 ring-amber-400/80">
          <CheckIcon size={12} />
        </span>
      );
    case "update_required":
      return <AlertIcon size={16} data-icon="warning" />;
    case "conflicts":
      return <DeclinedIcon size={16} data-icon="x" />;
    case "checking":
    case "loading":
      return (
        <span
          data-icon="spinner"
          role="presentation"
          className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-slate-600 border-t-slate-300"
        />
      );
    case "closed":
      return <DeclinedIcon size={16} data-icon="closed" />;
    default: {
      const never: never = state;
      return never;
    }
  }
}

/** The Branch summary card: conflicts, and whether the head is behind its base. */
export function BranchCard({ branch, onOpen }: { branch: BranchController; onOpen: (url: string) => void }) {
  const { view } = branch;
  const text = branchText(view);
  return (
    <div data-testid="branch-card" data-branch-state={view.state} aria-label={`Branch: ${text.value}, ${text.caption}`} className={CARD}>
      <p className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-slate-500">
        <GitBranchIcon size={13} />
        Branch
      </p>
      <p className={`${VALUE} flex items-center gap-2 ${TONE_TEXT[text.tone]}`}>
        <StateIcon state={view.state} />
        <span className="truncate">{text.value}</span>
      </p>
      <span aria-hidden className={BAR_SLOT} />
      <p className={CAPTION}>{text.caption}</p>
      <BranchAction branch={branch} onOpen={onOpen} className="mt-2" />
    </div>
  );
}

/** Update branch, or Resolve on GitHub, depending on the state; nothing otherwise. */
export function BranchAction({ branch, onOpen, className = "" }: { branch: BranchController; onOpen: (url: string) => void; className?: string }) {
  const { view } = branch;
  if (view.state === "conflicts" && view.conflictsUrl) {
    return (
      <div className={className}>
        <button
          type="button"
          onClick={() => onOpen(view.conflictsUrl)}
          className="inline-flex items-center gap-1 rounded-md border border-rose-500/40 px-2 py-0.5 text-xs text-rose-200 hover:bg-rose-500/10"
        >
          Resolve on GitHub
          <ExternalIcon size={11} />
        </button>
      </div>
    );
  }
  if (view.state === "behind" || view.state === "update_required") {
    return (
      <div className={className}>
        <UpdateBranchButton base={view.base} busy={branch.updating} onChoose={branch.choose} />
      </div>
    );
  }
  return null;
}

/** A split button: the face merges the base in; the chevron offers Merge or Rebase. */
export function UpdateBranchButton({ base, busy, onChoose }: { base: string; busy: UpdateMethod | null; onChoose: (method: UpdateMethod) => void }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) {
      return;
    }
    const onDown = (event: MouseEvent) => wrap.current && !wrap.current.contains(event.target as Node) && setOpen(false);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.stopPropagation();
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open]);
  const pick = (method: UpdateMethod) => {
    setOpen(false);
    onChoose(method);
  };
  const face = "border border-slate-700 bg-slate-800/60 px-2 py-0.5 text-xs text-slate-100 enabled:hover:bg-slate-700 disabled:opacity-60";
  return (
    <div ref={wrap} className="relative inline-flex">
      <button type="button" disabled={busy !== null} onClick={() => pick("merge")} className={`rounded-l-md ${face}`}>
        {busy === "rebase" ? "Rebasing…" : busy === "merge" ? "Updating…" : "Update branch"}
      </button>
      <button
        type="button"
        aria-label="Update branch options"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={busy !== null}
        onClick={() => setOpen((value) => !value)}
        className={`-ml-px rounded-r-md px-1 ${face}`}
      >
        <ChevronDownIcon size={12} />
      </button>
      {open && (
        <div
          role="menu"
          aria-label="Update branch"
          className="absolute right-0 top-full z-20 mt-1 w-60 rounded-md border border-slate-700 bg-slate-900 py-1 text-xs shadow-xl"
        >
          <button type="button" role="menuitem" onClick={() => pick("merge")} className="block w-full px-3 py-1.5 text-left hover:bg-slate-800">
            <span className="text-slate-100">Merge {base} into branch</span>
            <span className="ml-1 text-slate-500">(default)</span>
            <span className="block text-slate-500">Adds a merge commit; history is kept.</span>
          </button>
          <button type="button" role="menuitem" onClick={() => pick("rebase")} className="block w-full px-3 py-1.5 text-left hover:bg-slate-800">
            <span className="text-slate-100">Rebase</span>
            <span className="block text-slate-500">Replays the commits on {base}; rewrites history.</span>
          </button>
        </div>
      )}
    </div>
  );
}

/** Rebasing force-pushes new commits, so it is confirmed first, louder when others committed. */
export function RebaseConfirm({ branch, headRef }: { branch: BranchController; headRef: string }) {
  if (!branch.confirmingRebase) {
    return null;
  }
  const others = branch.view.otherAuthors;
  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center bg-black/50 p-8"
      onMouseDown={(event) => event.target === event.currentTarget && branch.cancelRebase()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="rebase-title"
        className="mt-24 w-full max-w-md rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <h2 id="rebase-title" className="text-base font-semibold">
          Rebase {headRef} onto {branch.view.base}?
        </h2>
        <p className="mt-2 text-slate-300">
          Rebasing rewrites the branch history: every commit gets a new SHA and the branch is force-updated. Anyone with a local copy has to reset it, and
          existing review comments may point at outdated commits.
        </p>
        {others.length > 0 && (
          <p role="alert" className="mt-3 flex gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-amber-100">
            <AlertIcon size={16} className="mt-0.5 shrink-0" />
            <span>
              This branch has commits by {others.map((login) => `@${login}`).join(", ")}. Rebasing rewrites their commits too; tell them before you do it.
            </span>
          </p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button type="button" onClick={branch.cancelRebase} className="rounded-md px-3 py-1 text-slate-300 hover:bg-slate-800">
            Cancel
          </button>
          <button type="button" onClick={branch.confirmRebase} className="rounded-md bg-amber-600 px-3 py-1 font-semibold text-white hover:bg-amber-500">
            Rebase branch
          </button>
        </div>
      </div>
    </div>
  );
}

/** A failed update, in the same banner style as the sync problems. */
export function BranchError({ branch }: { branch: BranchController }) {
  if (!branch.error) {
    return null;
  }
  return (
    <div
      role="alert"
      data-branch-error=""
      className="flex items-start gap-2 rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-100"
    >
      <AlertIcon size={14} className="mt-0.5 shrink-0" />
      <span className="min-w-0 flex-1">{branch.error}</span>
      <CloseButton label="Dismiss" shortcut={null} onClick={branch.dismissError} tone="text-rose-200 hover:text-white" />
    </div>
  );
}
