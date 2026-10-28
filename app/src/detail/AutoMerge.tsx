import { useState, type ReactElement } from "react";
import type { Api, MergeMethod, PrOverview } from "../api";
import { errorMessage } from "../api";
import { Tooltip } from "../Tooltip";
import { autoMergeView, canMergeNow, METHOD_LABEL } from "./status";

interface Props {
  api: Api;
  prId: number;
  overview: PrOverview;
  onChange: (next: PrOverview) => void;
  /** A short toast for the outcome, success or failure. */
  onNotice: (text: string) => void;
}

/**
 * The merge controls: the method, a direct Merge (confirmed first) when it can
 * merge now, and GitHub auto-merge on or off (no confirmation).
 */
export function AutoMerge({ api, prId, overview, onChange, onNotice }: Props) {
  const view = autoMergeView(overview);
  const [method, setMethod] = useState<MergeMethod | null>(overview.auto_merge?.method ?? view.defaultMethod);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [merging, setMerging] = useState(false);
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

  function merge(next: MergeMethod) {
    setMerging(true);
    api
      .mergePr(prId, next)
      .then((fresh) => {
        onChange(fresh);
        onNotice(`Merged: ${METHOD_LABEL[next].toLowerCase()} done.`);
      })
      .catch((err: unknown) => onNotice(`Could not merge: ${errorMessage(err)}`))
      .finally(() => {
        setMerging(false);
        setConfirming(false);
      });
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

  const mergeNow = canMergeNow(overview);
  const select = view.methods.length > 1 && (
    <select
      aria-label="Merge method"
      value={chosen ?? ""}
      disabled={(!view.available && !mergeNow) || busy || merging}
      onChange={(event) => {
        setMethod(event.target.value as MergeMethod);
        setConfirming(false);
      }}
      className="rounded-md border border-slate-700 bg-slate-900 px-1.5 py-1 text-xs text-slate-200 disabled:text-slate-500"
    >
      {view.methods.map((item) => (
        <option key={item} value={item}>
          {METHOD_LABEL[item]}
        </option>
      ))}
    </select>
  );
  const enable = (
    <button
      type="button"
      disabled={!view.available || busy || !chosen}
      onClick={() => chosen && run(chosen)}
      className={`rounded-md px-2.5 py-1 text-xs font-medium ${
        mergeNow ? "border border-violet-500/50 text-violet-200 enabled:hover:bg-violet-500/10" : "bg-violet-600/80 text-white enabled:hover:bg-violet-500"
      } disabled:border-slate-700 disabled:bg-slate-800 disabled:text-slate-500`}
    >
      {busy ? "Enabling…" : "Enable auto-merge"}
    </button>
  );
  return (
    <div className="flex flex-wrap items-center gap-2">
      {select}
      {mergeNow && chosen && (
        <MergeNow method={chosen} confirming={confirming} merging={merging} onAsk={() => setConfirming(true)} onCancel={() => setConfirming(false)} onConfirm={() => merge(chosen)} />
      )}
      <div aria-label="Auto-merge" className={view.available ? "" : "opacity-60"}>
        {view.available ? enable : <Disabled reason={view.reason}>{enable}</Disabled>}
      </div>
    </div>
  );
}

/** Merging can't be undone, so the first click only asks; the second merges. */
function MergeNow({ method, confirming, merging, onAsk, onCancel, onConfirm }: { method: MergeMethod; confirming: boolean; merging: boolean; onAsk: () => void; onCancel: () => void; onConfirm: () => void }) {
  if (!confirming) {
    return (
      <Tooltip label={METHOD_LABEL[method]} detail="Merges on GitHub now. You'll confirm first." side="top" align="end">
        <button type="button" onClick={onAsk} className="rounded-md bg-teal-600 px-2.5 py-1 text-xs font-semibold text-white hover:bg-teal-500">
          Merge
        </button>
      </Tooltip>
    );
  }
  return (
    <span role="group" aria-label="Confirm merge" className="flex items-center gap-1.5 rounded-md border border-teal-500/50 bg-teal-500/10 px-2 py-0.5 text-xs">
      <span className="text-teal-100">{METHOD_LABEL[method]} now? This can't be undone.</span>
      <button type="button" disabled={merging} onClick={onConfirm} className="rounded bg-teal-600 px-2 py-0.5 font-semibold text-white enabled:hover:bg-teal-500 disabled:opacity-60">
        {merging ? "Merging…" : "Confirm merge"}
      </button>
      <button type="button" disabled={merging} onClick={onCancel} className="rounded px-1.5 py-0.5 text-slate-300 hover:bg-slate-800">
        Cancel
      </button>
    </span>
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
