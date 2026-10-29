import { useCallback, useEffect, useState } from "react";
import type { Api, McpActivityEntry, McpOutcome } from "../api";
import { errorMessage } from "../api";
import { ago } from "../list/model";
import { Tooltip } from "../Tooltip";

interface Props {
  api: Pick<Api, "mcpActivity" | "clearMcpActivity">;
}

/** How often the list re-reads while open, so a running agent shows up. */
export const ACTIVITY_POLL_MS = 5_000;

const FILTERS: { value: McpOutcome | "all"; label: string }[] = [
  { value: "all", label: "All" },
  { value: "allowed", label: "Allowed" },
  { value: "denied", label: "Denied" },
  { value: "error", label: "Errors" },
];

const BADGE: Record<McpOutcome, string> = {
  allowed: "border-emerald-700/60 bg-emerald-950/40 text-emerald-200",
  denied: "border-amber-700/60 bg-amber-950/40 text-amber-200",
  error: "border-rose-700/60 bg-rose-950/40 text-rose-200",
};

const button = "rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800 disabled:opacity-50";

function Badge({ entry }: { entry: McpActivityEntry }) {
  const badge = (
    <span tabIndex={entry.error ? 0 : undefined} className={`shrink-0 rounded border px-1.5 py-0.5 text-[11px] leading-none ${BADGE[entry.outcome]}`}>
      {entry.outcome}
    </span>
  );
  return entry.error ? (
    <Tooltip label={entry.outcome === "denied" ? "Refused" : "Failed"} detail={entry.error} align="end">
      {badge}
    </Tooltip>
  ) : (
    badge
  );
}

/** Settings → Agents → Activity: every tool call an agent made, newest first. */
export function McpActivity({ api }: Props) {
  const [filter, setFilter] = useState<McpOutcome | "all">("all");
  const [entries, setEntries] = useState<McpActivityEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [now, setNow] = useState(() => new Date());

  const load = useCallback(
    () =>
      api
        .mcpActivity(filter === "all" ? undefined : filter)
        .then((next) => {
          setEntries(next);
          setNow(new Date());
          setError(null);
        })
        .catch((err: unknown) => setError(errorMessage(err))),
    [api, filter],
  );

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(), ACTIVITY_POLL_MS);
    return () => window.clearInterval(timer);
  }, [load]);

  function clear() {
    setConfirming(false);
    api
      .clearMcpActivity()
      .then(() => load())
      .catch((err: unknown) => setError(errorMessage(err)));
  }

  return (
    <section aria-labelledby="agents-activity">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="agents-activity" className="text-base font-semibold">
          Activity
        </h2>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1 text-slate-400">
            Show
            <select
              aria-label="Filter by outcome"
              className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-slate-200"
              value={filter}
              onChange={(event) => setFilter(event.target.value as McpOutcome | "all")}
            >
              {FILTERS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
          {!confirming && (
            <button type="button" className={button} disabled={!entries?.length && filter === "all"} onClick={() => setConfirming(true)}>
              Clear log…
            </button>
          )}
        </div>
      </div>
      <p className="mt-1 max-w-prose text-slate-400">
        Every tool call agents made, allowed or not. Kept on this computer: the last 5,000 calls, 90 days at most. Notes and other text agents send are never
        stored here.
      </p>
      {confirming && (
        <div
          role="group"
          aria-label="Confirm clear log"
          className="mt-2 flex flex-wrap items-center gap-2 rounded border border-amber-700/60 bg-amber-950/30 p-2"
        >
          <span className="text-amber-100">Delete every Activity entry? This cannot be undone.</span>
          <button type="button" className={`${button} border-amber-600`} onClick={clear}>
            Clear log
          </button>
          <button type="button" className={button} onClick={() => setConfirming(false)}>
            Cancel
          </button>
        </div>
      )}
      {error && (
        <p role="alert" className="mt-2 text-rose-300">
          Could not read the activity: {error}
        </p>
      )}
      {entries && entries.length === 0 && <p className="mt-3 text-slate-400">{filter === "all" ? "No agent has called a tool yet." : "Nothing matches."}</p>}
      {entries && entries.length > 0 && (
        <ul aria-label="Agent activity" className="mt-3 divide-y divide-slate-800 rounded border border-slate-800">
          {entries.map((entry) => (
            <li key={entry.id} className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
              <time dateTime={entry.at} className="w-16 shrink-0 text-xs text-slate-400">
                {ago(entry.at, now)}
              </time>
              <span className="min-w-0 max-w-[10rem] truncate text-xs text-slate-300">{entry.client || "unknown client"}</span>
              <span className="shrink-0 font-mono text-xs text-sky-300">{entry.tool}</span>
              <span className="min-w-0 flex-1 basis-40 truncate">{entry.summary}</span>
              <Badge entry={entry} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
