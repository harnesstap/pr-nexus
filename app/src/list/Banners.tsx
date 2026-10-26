import { useEffect, useState } from "react";
import type { Api, UpdateInfo } from "../api";
import { dismissMappingHint, dismissUpdate, updateDismissed } from "./hints";

/** "Update available": asks GitHub once per launch; any failure stays silent (About has the manual check). */
export function UpdateBanner({ api }: { api: Api }) {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancel = false;
    api
      .checkUpdate()
      .then((next) => {
        if (!cancel && next && !updateDismissed(next.version)) {
          setUpdate(next);
        }
      })
      .catch(() => undefined);
    return () => {
      cancel = true;
    };
  }, [api]);

  if (!update) {
    return null;
  }
  return (
    <div role="status" className="flex flex-wrap items-center gap-3 border-b border-violet-500/40 bg-violet-500/10 px-4 py-2 text-sm text-violet-100">
      <span>PR-Nexus {update.version} is available.</span>
      <button type="button" className="underline" onClick={() => void api.openExternal(update.url)}>
        Download from the release page ↗
      </button>
      <button
        type="button"
        className="underline"
        onClick={() => {
          void navigator.clipboard
            ?.writeText(update.command)
            .then(() => setCopied(true))
            .catch(() => setCopied(false));
        }}
      >
        {copied ? "Command copied" : "Copy gh command"}
      </button>
      <button
        type="button"
        aria-label="Dismiss update notice"
        className="ml-auto text-violet-300 hover:text-white"
        onClick={() => {
          dismissUpdate(update.version);
          setUpdate(null);
        }}
      >
        ×
      </button>
    </div>
  );
}

export function MappingHint({ onOpen, onDismiss }: { onOpen: () => void; onDismiss: () => void }) {
  return (
    <div className="flex flex-wrap items-center gap-3 border-b border-slate-800 bg-slate-900 px-4 py-2 text-sm text-slate-200">
      <span>No team channels mapped yet, so review requests ask for a channel each time.</span>
      <button type="button" className="text-violet-300 underline" onClick={onOpen}>
        Map team channels
      </button>
      <button
        type="button"
        aria-label="Dismiss mapping hint"
        className="ml-auto text-slate-500 hover:text-slate-200"
        onClick={() => {
          dismissMappingHint();
          onDismiss();
        }}
      >
        ×
      </button>
    </div>
  );
}
