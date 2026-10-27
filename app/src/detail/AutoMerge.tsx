import { useState, type ReactElement } from "react";
import type { Api, MergeMethod, PrOverview } from "../api";
import { errorMessage } from "../api";
import { Tooltip } from "../Tooltip";
import { autoMergeView, METHOD_LABEL } from "./status";

interface Props {
  api: Api;
  prId: number;
  overview: PrOverview;
  onChange: (next: PrOverview) => void;
  /** A short toast for the outcome, success or failure. */
  onNotice: (text: string) => void;
}

/** Turns GitHub auto-merge on (with a method the repository allows) or off. No confirmation. */
export function AutoMerge({ api, prId, overview, onChange, onNotice }: Props) {
  const view = autoMergeView(overview);
  const [method, setMethod] = useState<MergeMethod | null>(overview.auto_merge?.method ?? view.defaultMethod);
  const [busy, setBusy] = useState(false);
  const chosen = method && view.methods.includes(method) ? method : view.defaultMethod;

  function run(next: MergeMethod | null) {
    setBusy(true);
    api
      .setAutoMerge(prId, next)
      .then((fresh) => {
        onChange(fresh);
        onNotice(next ? `Auto-merge on: it will ${METHOD_LABEL[next].toLowerCase()} when ready.` : "Auto-merge turned off.");
      })
      .catch((err: unknown) => onNotice(`Could not change auto-merge: ${errorMessage(err)}`))
      .finally(() => setBusy(false));
  }

  if (view.enabled && overview.auto_merge) {
    const by = overview.auto_merge.enabled_by;
    const button = (
      <button
        type="button"
        disabled={!view.available || busy}
        onClick={() => run(null)}
        className="rounded-md border border-slate-700 px-2.5 py-1 text-xs text-slate-200 enabled:hover:bg-slate-800 disabled:opacity-50"
      >
        {busy ? "Turning off…" : "Disable"}
      </button>
    );
    return (
      <div className="flex items-center gap-2" aria-label="Auto-merge">
        <span className="flex items-center gap-1.5 rounded-full bg-teal-500/15 px-2 py-0.5 text-xs text-teal-200">
          <span className="h-1.5 w-1.5 rounded-full bg-teal-300" aria-hidden="true" />
          Auto-merge on · {METHOD_LABEL[overview.auto_merge.method]}
          {by ? ` · by @${by}` : ""}
        </span>
        {view.available ? button : <Disabled reason={view.reason}>{button}</Disabled>}
      </div>
    );
  }

  const controls = (
    <div className="flex items-stretch overflow-hidden rounded-md border border-violet-500/50">
      <button
        type="button"
        disabled={!view.available || busy || !chosen}
        onClick={() => chosen && run(chosen)}
        className="bg-violet-600/80 px-2.5 py-1 text-xs font-medium text-white enabled:hover:bg-violet-500 disabled:bg-slate-800 disabled:text-slate-500"
      >
        {busy ? "Enabling…" : "Enable auto-merge"}
      </button>
      {view.methods.length > 1 && (
        <select
          aria-label="Merge method"
          value={chosen ?? ""}
          disabled={!view.available || busy}
          onChange={(event) => setMethod(event.target.value as MergeMethod)}
          className="border-l border-violet-500/50 bg-slate-900 px-1.5 text-xs text-slate-200 disabled:text-slate-500"
        >
          {view.methods.map((item) => (
            <option key={item} value={item}>
              {METHOD_LABEL[item]}
            </option>
          ))}
        </select>
      )}
    </div>
  );
  return (
    <div aria-label="Auto-merge" className={view.available ? "" : "opacity-60"}>
      {view.available ? controls : <Disabled reason={view.reason}>{controls}</Disabled>}
    </div>
  );
}

/** Disabled controls get no hover events, so the tooltip hangs on a focusable wrapper. */
function Disabled({ reason, children }: { reason: string | null; children: ReactElement }) {
  return (
    <Tooltip label="Auto-merge unavailable" detail={reason ?? undefined} side="top" align="end">
      <span tabIndex={0} data-testid="auto-merge-disabled" className="inline-flex rounded-md outline-none focus-visible:ring-2 focus-visible:ring-violet-400">
        {children}
      </span>
    </Tooltip>
  );
}
