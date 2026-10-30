import { useEffect, useRef, useState } from "react";
import type { Api, UpdateInfo } from "../api";
import { ArrowUpCircleIcon, CopyIcon, ExternalIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { dismissUpdate, updateDismissed } from "./hints";

/** Re-check this often while the app stays open (plus once at startup). */
export const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

export function updateSummary(update: UpdateInfo): string {
  if (update.kind === "dev") {
    const n = update.behind_by ?? 0;
    return `${n} ${n === 1 ? "commit" : "commits"} behind main`;
  }
  return `PR Nexus ${update.version} is available`;
}

function versionLine(update: UpdateInfo): string {
  return update.kind === "dev" ? `Built from ${update.current} · main is at ${update.version}` : `You have ${update.current} · latest is ${update.version}`;
}

/**
 * The sidebar's "update available" icon, right of Settings. Asks GitHub at
 * startup and every 6 h; up to date, offline or a failed check shows nothing.
 */
export function UpdateIndicator({ api }: { api: Api }) {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let cancel = false;
    const check = () =>
      api
        .checkUpdate()
        .then((next) => {
          if (!cancel) {
            setUpdate(next && !updateDismissed(next.version) ? next : null);
          }
        })
        .catch((err: unknown) => console.warn("update check failed", err));
    void check();
    const timer = window.setInterval(() => void check(), UPDATE_CHECK_INTERVAL_MS);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, [api]);

  if (!update) {
    return null;
  }
  return (
    <>
      <Tooltip label={updateSummary(update)} detail={versionLine(update)} side="top" align="end">
        <button
          type="button"
          data-update-trigger=""
          aria-label={`${updateSummary(update)}, update details`}
          aria-expanded={open}
          className="shrink-0 rounded p-1 text-violet-400 hover:bg-slate-800 hover:text-violet-300"
          onClick={() => setOpen((value) => !value)}
        >
          <ArrowUpCircleIcon />
        </button>
      </Tooltip>
      {open && (
        <UpdatePopover
          api={api}
          update={update}
          onClose={() => setOpen(false)}
          onSkip={() => {
            dismissUpdate(update.version);
            setOpen(false);
            setUpdate(null);
          }}
        />
      )}
    </>
  );
}

export function UpdatePopover({ api, update, onClose, onSkip }: { api: Api; update: UpdateInfo; onClose: () => void; onSkip: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const close = useRef(onClose);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    close.current = onClose;
  }, [onClose]);
  useEffect(() => {
    ref.current?.focus();
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && close.current();
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (ref.current && target && !ref.current.contains(target) && !(target as Element).closest?.("[data-update-trigger]")) {
        close.current();
      }
    };
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
    };
  }, []);
  const dev = update.kind === "dev";
  return (
    <div
      ref={ref}
      role="dialog"
      aria-label="Update available"
      tabIndex={-1}
      className="absolute bottom-full left-2 z-30 mb-2 w-80 space-y-2 rounded-lg border border-slate-700 bg-slate-900 p-3 text-xs shadow-xl outline-none"
    >
      <p className="font-medium text-slate-100">{updateSummary(update)}</p>
      <p className="text-slate-400">{versionLine(update)}</p>
      {dev && (
        <>
          <p className="text-slate-300">From the repo root:</p>
          <div className="flex items-start gap-1 rounded bg-slate-950 px-2 py-1.5">
            <code className="min-w-0 flex-1 break-words font-mono text-[11px] text-slate-200">{update.command}</code>
            <button
              type="button"
              aria-label="Copy command"
              className="shrink-0 rounded p-0.5 text-slate-400 hover:text-slate-200"
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(update.command)
                  .then(() => setCopied(true))
                  .catch(() => setCopied(false));
              }}
            >
              <CopyIcon size={12} />
            </button>
          </div>
          {copied && (
            <p role="status" className="text-slate-400">
              Command copied.
            </p>
          )}
        </>
      )}
      <div className="flex items-center gap-2 pt-1">
        <button
          type="button"
          className="flex items-center gap-1 whitespace-nowrap rounded bg-violet-600 px-2 py-1 font-medium text-white hover:bg-violet-500"
          onClick={() => void api.openExternal(update.url)}
        >
          {dev ? "View changes" : "Open release page"}
          <ExternalIcon size={12} />
        </button>
        <button type="button" className="ml-auto whitespace-nowrap rounded px-2 py-1 text-slate-400 hover:bg-slate-800 hover:text-slate-200" onClick={onSkip}>
          {dev ? "Skip this commit" : "Skip this version"}
        </button>
      </div>
    </div>
  );
}
