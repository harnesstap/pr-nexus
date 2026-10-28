import type { ReactNode } from "react";

/**
 * The non-blocking "still a draft" callout of the Slack dialogs, in the merge
 * card's warning yellow. Mark ready uses the same `setPrDraft` call as the
 * merge card; sending stays possible either way.
 */
export function DraftWarning({
  children,
  busy,
  onMarkReady,
  action = "Mark ready for review",
}: {
  children: ReactNode;
  busy: boolean;
  onMarkReady?: () => void;
  action?: string;
}) {
  return (
    <div
      role="note"
      aria-label="Draft warning"
      className="mt-3 flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5 rounded-md border border-amber-400/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-100"
    >
      <div className="min-w-0 flex-1">{children}</div>
      {onMarkReady && (
        <button
          type="button"
          disabled={busy}
          onClick={onMarkReady}
          className="shrink-0 rounded-md bg-violet-500/20 px-2 py-0.5 text-xs font-medium text-violet-100 ring-1 ring-violet-400/40 enabled:hover:bg-violet-500/30 disabled:cursor-default disabled:opacity-50"
        >
          {busy ? "Saving…" : action}
        </button>
      )}
    </div>
  );
}
