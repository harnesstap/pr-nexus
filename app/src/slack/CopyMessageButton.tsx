import { useEffect, useState } from "react";
import { CheckIcon, CopyIcon } from "../icons";
import { copyText } from "../list/markdown";
import type { Toast } from "../list/model";
import { Tooltip } from "../Tooltip";

export const COPIED_MS = 1500;

/**
 * Copies one planned message so it can be posted by hand. Copying is not
 * sending: nothing is posted, tracked or recorded, and the dialog stays open.
 */
export function CopyMessageButton({ text, onNotice }: { text: string; onNotice?: (toast: Toast) => void }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) {
      return;
    }
    const timer = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  function copy() {
    copyText(text).then(
      () => {
        setCopied(true);
        onNotice?.({ text: "Message copied" });
      },
      (reason: unknown) => onNotice?.({ text: `Couldn't copy to the clipboard: ${reason instanceof Error ? reason.message : String(reason)}`, error: true }),
    );
  }

  return (
    <div className="absolute right-1.5 top-1.5">
      <Tooltip label={copied ? "Copied" : "Copy message"} align="end">
        <button
          type="button"
          aria-label="Copy message"
          className="cursor-pointer rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 focus-visible:outline focus-visible:outline-2 focus-visible:outline-violet-400"
          onClick={copy}
        >
          {copied ? <CheckIcon size={14} className="text-emerald-300" /> : <CopyIcon size={14} />}
        </button>
      </Tooltip>
    </div>
  );
}
