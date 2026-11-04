import { useCallback, useEffect, useState } from "react";
import type { Api, DurationSummary, RequestTarget, ReviewStatsView, StatsFilter, StatsPeriod } from "../api";
import { errorMessage } from "../api";

interface Props {
  api: Pick<Api, "reviewStats">;
}

const PERIODS: StatsPeriod[] = [7, 30, 90];

const button = "cursor-pointer rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800 disabled:cursor-default disabled:opacity-50";

/** `3h 20m`, `45m`, `2d 4h`, or `—` when there is nothing to show. */
export function formatDuration(secs: number | null): string {
  if (secs === null) {
    return "—";
  }
  const minutes = Math.round(secs / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const rest = minutes % 60;
    return rest ? `${hours}h ${rest}m` : `${hours}h`;
  }
  const rest = hours % 24;
  return rest ? `${Math.floor(hours / 24)}d ${rest}h` : `${Math.floor(hours / 24)}d`;
}

function targetLabel(target: RequestTarget): string {
  switch (target.kind) {
    case "user":
      return `@${target.name}`;
    case "team":
      return `@${target.name}`;
    case "slack_channel":
      return `#${target.name}`;
    default: {
      const never: never = target;
      return never;
    }
  }
}

function targetKind(target: RequestTarget): string {
  switch (target.kind) {
    case "user":
      return "GitHub";
    case "team":
      return "GitHub team";
    case "slack_channel":
      return "Slack";
    default: {
      const never: never = target;
      return never;
    }
  }
}

/** The repo / org menu's value: `org:acme`, `repo:acme/api` or empty for all. */
function filterFrom(value: string): StatsFilter {
  if (value.startsWith("org:")) {
    return { org: value.slice(4) };
  }
  if (value.startsWith("repo:")) {
    return { repo: value.slice(5) };
  }
  return {};
}

function Card({ label, value, detail }: { label: string; value: string; detail?: string }) {
  return (
    <div className="rounded border border-slate-800 bg-slate-900/60 px-3 py-2">
      <div className="text-xs text-slate-400">{label}</div>
      <div className="mt-0.5 text-lg font-semibold text-slate-100">{value}</div>
      {detail && <div className="text-xs text-slate-500">{detail}</div>}
    </div>
  );
}

function durationCard(label: string, summary: DurationSummary) {
  return (
    <Card
      label={label}
      value={formatDuration(summary.median_secs)}
      detail={summary.count ? `median · average ${formatDuration(summary.average_secs)} · ${summary.count} PR${summary.count === 1 ? "" : "s"}` : "no data yet"}
    />
  );
}

function Skeleton() {
  return (
    <div aria-label="Loading stats" aria-busy="true" className="mt-3 animate-pulse space-y-3">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {Array.from({ length: 6 }, (_, index) => (
          <div key={index} className="h-16 rounded border border-slate-800 bg-slate-900/60" />
        ))}
      </div>
      <div className="h-40 rounded border border-slate-800 bg-slate-900/60" />
    </div>
  );
}

/** Settings → Agents → Activity → Stats: how fast the user's PRs get reviewed, and by whom. */
export function ReviewStats({ api }: Props) {
  const [days, setDays] = useState<StatsPeriod>(30);
  const [scope, setScope] = useState("");
  const [view, setView] = useState<ReviewStatsView | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The request whose answer is on screen; loading while it differs.
  const [shown, setShown] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const key = `${days}|${scope}`;
  const loading = refreshing || shown !== key;

  const load = useCallback(
    (refresh: boolean) =>
      api
        .reviewStats(days, filterFrom(scope), refresh)
        .then((next) => {
          setView(next);
          setError(null);
        })
        .catch((err: unknown) => setError(errorMessage(err)))
        .finally(() => {
          setShown(`${days}|${scope}`);
          setRefreshing(false);
        }),
    [api, days, scope],
  );

  useEffect(() => {
    void load(false);
  }, [load]);

  function refresh() {
    setRefreshing(true);
    void load(true);
  }

  const stats = view?.stats;
  const orgs = [...new Set((stats?.repos ?? []).map((repo) => repo.split("/")[0]))];

  return (
    <div className="mt-3">
      <div className="flex flex-wrap items-center gap-2">
        <div role="group" aria-label="Period" className="flex overflow-hidden rounded border border-slate-700">
          {PERIODS.map((period) => (
            <button
              key={period}
              type="button"
              aria-pressed={days === period}
              className={`cursor-pointer px-2.5 py-1 text-sm ${days === period ? "bg-slate-700 text-slate-100" : "text-slate-300 hover:bg-slate-800"}`}
              onClick={() => setDays(period)}
            >
              {period}d
            </button>
          ))}
        </div>
        <select
          aria-label="Repository or organization"
          className="cursor-pointer rounded border border-slate-700 bg-slate-900 px-2 py-1 text-slate-200"
          value={scope}
          onChange={(event) => setScope(event.target.value)}
        >
          <option value="">All tracked repositories</option>
          {orgs.map((org) => (
            <option key={`org:${org}`} value={`org:${org}`}>
              {org} (all)
            </option>
          ))}
          {(stats?.repos ?? []).map((repo) => (
            <option key={`repo:${repo}`} value={`repo:${repo}`}>
              {repo}
            </option>
          ))}
        </select>
        <button type="button" className={`${button} ml-auto`} disabled={loading} onClick={refresh}>
          {loading ? "Loading…" : "Refresh"}
        </button>
      </div>
      {error && (
        <p role="alert" className="mt-2 text-rose-300">
          Could not load the stats: {error}
        </p>
      )}
      {view?.partial && <p className="mt-2 text-amber-200">Partial: GitHub's rate limit is low, so older pull requests were left out.</p>}
      {!stats && loading && <Skeleton />}
      {stats && (
        <div className={loading ? "opacity-60" : undefined}>
          <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
            {durationCard("Time to first review", stats.first_review)}
            {durationCard("Slack request → first review", stats.slack_to_first_review)}
            <Card label="Opened" value={String(stats.opened)} detail={`since ${new Date(stats.since).toLocaleDateString()}`} />
            <Card label="Merged" value={String(stats.merged)} />
            <Card label="Waiting for a first review" value={String(stats.waiting)} />
            <Card label="Slack reminders sent" value={String(stats.reminders)} />
          </div>
          <h3 className="mt-4 text-sm font-semibold text-slate-200">Most requested reviewers</h3>
          {stats.reviewers.length === 0 ? (
            <p className="mt-1 text-slate-400">No review requests in this period.</p>
          ) : (
            <table aria-label="Most requested reviewers" className="mt-2 w-full border-collapse overflow-hidden rounded border border-slate-800 text-sm">
              <thead className="bg-slate-900/60 text-left text-xs text-slate-400">
                <tr>
                  <th className="px-3 py-1.5 font-normal">Reviewer</th>
                  <th className="px-3 py-1.5 text-right font-normal">Requests</th>
                  <th className="px-3 py-1.5 text-right font-normal">Response rate</th>
                  <th className="px-3 py-1.5 text-right font-normal">Median response</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-800">
                {stats.reviewers.map((reviewer) => (
                  <tr key={`${reviewer.target.kind}:${reviewer.target.name}`}>
                    <td className="px-3 py-1.5">
                      <span className="font-mono text-sky-300">{targetLabel(reviewer.target)}</span>
                      <span className="ml-2 text-xs text-slate-500">{targetKind(reviewer.target)}</span>
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums">{reviewer.requests}</td>
                    <td className="px-3 py-1.5 text-right tabular-nums">
                      {Math.round(reviewer.response_rate * 100)}% <span className="text-xs text-slate-500">({reviewer.responded})</span>
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums">{formatDuration(reviewer.median_response_secs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <p className="mt-2 text-xs text-slate-500">
            Your pull requests created in the period. Time to first review runs from ready for review to the first review by someone else; bots are left out.
          </p>
        </div>
      )}
    </div>
  );
}
