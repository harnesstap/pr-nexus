import { useEffect, useRef } from "react";
import type { SlackWorkspace, SyncStatus } from "../api";
import { ago, liveLabel, syncLabel } from "./model";

/** What the sync error banner says, or null when sync is fine (§6). */
export interface SyncProblem {
  /** `auth`: GitHub rejected the token (401); offer Reconnect GitHub. */
  kind: "auth" | "error";
  title: string;
  detail: string;
  /** "data from 3h ago", or null when nothing was ever synced. */
  age: string | null;
}

const AUTH_PATTERN = /\b401\b|unauthori[sz]ed|bad credentials|token (?:expired|revoked|invalid)|requires authentication/i;

export function syncProblem(sync: SyncStatus, now: Date): SyncProblem | null {
  if (sync.phase !== "error") {
    return null;
  }
  const reason = sync.message.replace(/^Sync failed · /, "");
  const age = sync.synced_at ? `data from ${ago(sync.synced_at, now)}` : null;
  if (AUTH_PATTERN.test(reason)) {
    return { kind: "auth", title: "GitHub sign-in expired. The list is not updating.", detail: reason, age };
  }
  return { kind: "error", title: "GitHub sync failed. The list may be out of date.", detail: reason, age };
}

/** Enabled workspaces whose Slack sign-in is gone; disabled ones are never used, so they stay quiet. */
export function workspacesNeedingSignIn(workspaces: SlackWorkspace[]): SlackWorkspace[] {
  return [...workspaces].sort((a, b) => a.priority - b.priority).filter((item) => item.enabled && item.needs_sign_in);
}

/**
 * Why Request review / Bump is off, or null. A bump goes to the thread's workspace;
 * a new request is blocked only when every enabled workspace needs sign-in.
 */
export function slackActionBlock(workspaces: SlackWorkspace[], threadWorkspace?: string | null): string | null {
  const stale = workspacesNeedingSignIn(workspaces);
  if (threadWorkspace) {
    const hit = stale.find((item) => item.domain === threadWorkspace || item.team_id === threadWorkspace);
    return hit ? `${hit.team_name || hit.domain} Slack needs sign-in` : null;
  }
  const enabled = workspaces.filter((item) => item.enabled);
  if (enabled.length === 0 || stale.length < enabled.length) {
    return null;
  }
  return `${stale.map((item) => item.team_name || item.domain).join(", ")} Slack needs sign-in`;
}

const BANNER = "flex flex-wrap items-center gap-3 border-b px-4 py-2 text-sm";

/** Red banner over the list while GitHub sync fails, so stale rows are not mistaken for fresh ones. */
export function SyncErrorBanner({ sync, now, onReconnect, onRetry }: { sync: SyncStatus; now: Date; onReconnect: () => void; onRetry: () => void }) {
  const problem = syncProblem(sync, now);
  if (!problem) {
    return null;
  }
  return (
    <div role="alert" data-sync-problem={problem.kind} className={`${BANNER} border-rose-500/40 bg-rose-500/10 text-rose-100`}>
      <span className="font-medium">{problem.title}</span>
      <span className="text-rose-200/80">
        {problem.detail}
        {problem.age && ` · ${problem.age}`}
      </span>
      <span className="ml-auto flex gap-3">
        {problem.kind === "auth" ? (
          <button type="button" className="rounded bg-rose-500/80 px-2.5 py-1 font-medium text-white hover:bg-rose-500" onClick={onReconnect}>
            Reconnect GitHub
          </button>
        ) : (
          <button type="button" className="underline" onClick={onRetry}>
            Try again
          </button>
        )}
      </span>
    </div>
  );
}

/** Amber banner when an enabled Slack workspace needs sign-in: requests and bumps there would fail. */
export function SlackSignInBanner({ workspaces, onReconnect }: { workspaces: SlackWorkspace[]; onReconnect: () => void }) {
  const stale = workspacesNeedingSignIn(workspaces);
  if (stale.length === 0) {
    return null;
  }
  const names = stale.map((item) => item.team_name || item.domain).join(", ");
  return (
    <div role="alert" data-slack-sign-in="needed" className={`${BANNER} border-amber-500/40 bg-amber-500/10 text-amber-100`}>
      <span>
        {names} Slack {stale.length === 1 ? "needs" : "need"} sign-in · review requests and bumps there are paused.
      </span>
      <button type="button" className="underline" onClick={onReconnect}>
        Reconnect
      </button>
    </div>
  );
}

interface PopoverProps {
  sync: SyncStatus;
  now: Date;
  /** Open PRs in the current view's summary; null until the list loaded. */
  prCount: number | null;
  onSync: () => void;
  onFullResync: () => void;
  onClose: () => void;
}

/** The sync details over the sidebar footer: labeled rows and real buttons. */
export function SyncPopover({ sync, now, prCount, onSync, onFullResync, onClose }: PopoverProps) {
  const ref = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  const syncing = sync.phase === "syncing";
  useEffect(() => {
    close.current = onClose;
  }, [onClose]);
  useEffect(() => {
    ref.current?.focus();
    const onClose = () => close.current();
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && onClose();
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (ref.current && target && !ref.current.contains(target) && !(target as Element).closest?.("[data-sync-trigger]")) {
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, []);
  const live = liveLabel(sync, now) ?? (sync.live?.state === "demo" ? "Off in demo mode" : "Not available");
  const rows: [string, string][] = [
    ["Status", syncLabel(sync, now)],
    ["Last sync", sync.synced_at ? `${ago(sync.synced_at, now)} · ${new Date(sync.synced_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "Never"],
    ["Live updates", live],
    ["Open PRs", prCount === null ? "—" : String(prCount)],
  ];
  return (
    <div
      ref={ref}
      role="dialog"
      aria-label="Sync details"
      tabIndex={-1}
      className="absolute bottom-full left-2 right-2 z-30 mb-2 rounded-lg border border-slate-700 bg-slate-900 p-3 text-xs shadow-xl outline-none"
    >
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-slate-500">{label}</dt>
            <dd className={`min-w-0 break-words ${label === "Status" && sync.phase === "error" ? "text-rose-300" : "text-slate-200"}`}>{value}</dd>
          </div>
        ))}
      </dl>
      <div className="mt-3 flex gap-2">
        <button type="button" disabled={syncing} className="flex-1 rounded bg-violet-500 px-2 py-1.5 font-medium text-white hover:bg-violet-400 disabled:opacity-50" onClick={onSync}>
          {syncing ? "Syncing…" : "Sync now"}
        </button>
        <button
          type="button"
          disabled={syncing}
          title="Re-read every open PR from GitHub, not only what changed. Runs on its own at start and every 6 hours."
          className="flex-1 rounded border border-slate-600 px-2 py-1.5 text-slate-200 hover:bg-slate-800 disabled:opacity-50"
          onClick={onFullResync}
        >
          Full resync
        </button>
      </div>
    </div>
  );
}
