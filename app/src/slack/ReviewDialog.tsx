import { useEffect, useState } from "react";
import type { Api, ListRow, ReviewPreview, ReviewTarget, SlackIndicator, SlackWorkspace, TargetChoice } from "../api";
import { channelLabel, errorMessage } from "../api";

interface Props {
  api: Api;
  row: ListRow;
  onClose: () => void;
  /** Called with the stored request after a post or bump succeeded. */
  onSent: (indicator: SlackIndicator) => void;
  onOpen: (url: string) => void;
}

const MANUAL = "manual";

/**
 * Request or bump review on Slack for one PR (§7.11). Shows the exact text
 * before anything is posted; Send is the only call that reaches Slack.
 */
export function ReviewDialog({ api, row, onClose, onSent, onOpen }: Props) {
  const [teams, setTeams] = useState<string[]>([]);
  const [choices, setChoices] = useState<TargetChoice[] | null>(null);
  const [workspaces, setWorkspaces] = useState<SlackWorkspace[]>([]);
  const [selected, setSelected] = useState<string>(MANUAL);
  const [manual, setManual] = useState<ReviewTarget>({ workspace: "", channel_id: "", channel_name: "" });
  const [preview, setPreview] = useState<ReviewPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  /** The send went through but its link is unknown: never offer a resend. */
  const [unconfirmed, setUnconfirmed] = useState<string | null>(null);

  async function loadPreview(target: ReviewTarget) {
    setPreview(null);
    setError(null);
    try {
      setPreview(await api.reviewPreview(row.id, target));
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  useEffect(() => {
    // ListScreen keys this dialog by PR id, so state starts empty for each PR.
    let cancel = false;
    void (async () => {
      try {
        const [detail, signedIn] = await Promise.all([api.prDetail(row.id).catch(() => null), api.slackWorkspaces()]);
        const wanted = detail ? (detail.mandatory_teams.length > 0 ? detail.mandatory_teams : detail.candidate_teams) : [];
        const found = await api.reviewTargets(row.id, wanted);
        if (cancel) {
          return;
        }
        setTeams(wanted);
        setWorkspaces(signedIn);
        setChoices(found);
        const firstSigned = signedIn.find((item) => item.signed_in);
        setManual((current) => ({ ...current, workspace: firstSigned?.domain ?? signedIn[0]?.domain ?? "" }));
        if (found.length > 0) {
          setSelected("0");
          const next = await api.reviewPreview(row.id, found[0].target);
          if (!cancel) {
            setPreview(next);
          }
        }
      } catch (err) {
        if (!cancel) {
          setChoices((current) => current ?? []);
          setError(errorMessage(err));
        }
      }
    })();
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

  function choose(value: string) {
    setSelected(value);
    if (value === MANUAL) {
      setPreview(null);
      setError(null);
    } else if (choices) {
      void loadPreview(choices[Number(value)].target);
    }
  }

  async function send() {
    if (!preview) {
      return;
    }
    setSending(true);
    setError(null);
    try {
      const chosen = selected === MANUAL ? null : choices?.[Number(selected)];
      onSent(await api.sendReview(row.id, preview.target, chosen?.teams.length ? chosen.teams : teams));
    } catch (err) {
      if (err && typeof err === "object" && (err as { kind?: unknown }).kind === "sent_unconfirmed") {
        setUnconfirmed(errorMessage(err));
      } else {
        setError(errorMessage(err));
      }
      setSending(false);
    }
  }

  const input = "mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1.5";
  const bump = preview?.action === "bump";
  return (
    <div
      className="fixed inset-0 z-40 flex items-start justify-center overflow-auto bg-black/50 p-8"
      onMouseDown={(event) => event.target === event.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="review-dialog-title"
        className="w-full max-w-2xl rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl"
      >
        <h2 id="review-dialog-title" className="text-base font-semibold">
          {bump ? "Bump review request on Slack" : "Request review on Slack"}
        </h2>
        <p className="truncate text-xs text-slate-500">
          #{row.number} {row.title} · {row.repo}
        </p>
        {choices === null ? (
          <p className="mt-4 text-slate-400">Finding channels…</p>
        ) : (
          <div className="mt-4 space-y-3">
            <label className="block">
              Channel
              <select className={input} value={selected} onChange={(event) => choose(event.target.value)}>
                {choices.map((choice, index) => (
                  <option key={`${choice.target.workspace}/${choice.target.channel_id}`} value={String(index)}>
                    {choice.target.workspace} · {channelLabel(choice.target)}
                    {choice.source === "request" ? " (open request)" : choice.teams.length ? ` (${choice.teams.join(", ")})` : ""}
                  </option>
                ))}
                <option value={MANUAL}>Another channel…</option>
              </select>
            </label>
            {choices.length === 0 && (
              <p className="text-xs text-slate-400">No mapped channel for this PR's teams. Enter one below.</p>
            )}
            {selected === MANUAL && (
              <div className="grid grid-cols-3 gap-2">
                <label className="block">
                  Workspace
                  <select
                    className={input}
                    value={manual.workspace}
                    onChange={(event) => setManual({ ...manual, workspace: event.target.value })}
                  >
                    {workspaces.length === 0 && <option value="">No workspace connected</option>}
                    {workspaces.map((item) => (
                      <option key={item.id} value={item.domain}>
                        {item.team_name}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="block">
                  Channel or DM ID
                  <input
                    className={input}
                    placeholder="C0123456789 or D0123456789"
                    value={manual.channel_id}
                    onChange={(event) => setManual({ ...manual, channel_id: event.target.value })}
                  />
                </label>
                <label className="block">
                  Channel name
                  <input
                    className={input}
                    placeholder="#reviews"
                    value={manual.channel_name}
                    onChange={(event) => setManual({ ...manual, channel_name: event.target.value })}
                  />
                </label>
                <button
                  type="button"
                  className="col-span-3 justify-self-start rounded border border-slate-600 px-3 py-1.5"
                  onClick={() => void loadPreview(manual)}
                >
                  Preview
                </button>
              </div>
            )}
            {preview && (
              <div className="space-y-2">
                <p className="text-xs text-slate-400">
                  {bump
                    ? `Bump in the existing thread (${preview.bump_count} earlier bump${preview.bump_count === 1 ? "" : "s"}).`
                    : `New message in ${channelLabel(preview.target)}.`}
                  {bump && preview.permalink && (
                    <>
                      {" "}
                      <button type="button" className="underline" onClick={() => onOpen(preview.permalink!)}>
                        Open thread ↗
                      </button>
                    </>
                  )}
                </p>
                <pre aria-label="Message preview" className="whitespace-pre-wrap rounded bg-slate-950 p-3 text-xs">
                  {preview.text}
                </pre>
                {preview.warnings.map((warning) => (
                  <p key={warning} className="text-xs text-amber-300">
                    {warning}
                  </p>
                ))}
                {!preview.signed_in && (
                  <p className="text-xs text-amber-300">
                    Sign in to {preview.target.workspace} in Settings → Slack before sending.
                  </p>
                )}
              </div>
            )}
          </div>
        )}
        {error && <p className="mt-3 text-rose-300">{error}</p>}
        {unconfirmed && (
          <p className="mt-3 text-amber-300" role="status">
            {unconfirmed}
          </p>
        )}
        <footer className="mt-5 flex justify-end gap-2">
          <button type="button" className="rounded px-3 py-1.5 text-slate-300 hover:bg-slate-800" onClick={onClose}>
            {unconfirmed ? "Close" : "Cancel"}
          </button>
          <button
            type="button"
            className="rounded bg-violet-500 px-3 py-1.5 font-medium text-white disabled:opacity-40"
            disabled={!preview || !preview.signed_in || sending || unconfirmed !== null}
            onClick={() => void send()}
          >
            {sending ? "Sending…" : bump ? "Bump in thread" : "Send request"}
          </button>
        </footer>
      </div>
    </div>
  );
}
