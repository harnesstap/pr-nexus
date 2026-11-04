import { useLayoutEffect, useRef, type KeyboardEvent } from "react";
import { CloseButton } from "../CloseButton";
import type { ChangelogRelease } from "./changelog";
import { ReleaseNotes } from "./ReleaseNotes";

const FOCUSABLE = 'button:not([disabled]), a[href], input:not([disabled]), [tabindex]:not([tabindex="-1"])';

interface Props {
  current: string;
  releases: ChangelogRelease[];
  onClose: () => void;
  onFullChangelog: () => void;
}

/** "What's new" after an update: every release since the last one seen (§8.2). */
export function WhatsNewModal({ current, releases, onClose, onFullChangelog }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  // Layout effect: focus moves in the same commit that shows the dialog, so
  // nothing (a user or a test) can observe it opened with focus still on <body>.
  useLayoutEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    ref.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    return () => previous?.focus?.();
  }, []);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key !== "Tab" || !ref.current) {
      return;
    }
    const items = [...ref.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) {
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-8" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="whats-new-title"
        onKeyDown={onKeyDown}
        className="flex max-h-[calc(100vh-4rem)] w-full max-w-xl flex-col rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl outline-none"
      >
        <div className="flex items-center justify-between">
          <h2 id="whats-new-title" className="text-base font-semibold">
            What&apos;s new in PR Nexus {current.replace(/^v?/, "v")}
          </h2>
          <CloseButton label="Close what's new" onClick={onClose} />
        </div>
        <div className="mt-4 min-h-0 flex-1 space-y-3 overflow-auto pr-1">
          {releases.map((release) => (
            <ReleaseNotes key={release.version} release={release} />
          ))}
        </div>
        <div className="mt-4 flex items-center justify-between gap-3">
          <button type="button" className="text-violet-300 underline hover:text-violet-200" onClick={onFullChangelog}>
            See full changelog
          </button>
          <button type="button" data-autofocus className="rounded bg-violet-600 px-3 py-1.5 font-medium text-white hover:bg-violet-500" onClick={onClose}>
            Got it
          </button>
        </div>
      </div>
    </div>
  );
}
