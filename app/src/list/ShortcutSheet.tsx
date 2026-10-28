import { useEffect, useRef } from "react";
import { CloseButton } from "../CloseButton";
import { displayKey, groupedShortcuts } from "./shortcuts";

/** The `?` overlay: every shortcut from the registry, grouped. */
export function ShortcutSheet({ onClose }: { onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.focus();
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape" || event.key === "?") {
        event.preventDefault();
        onClose();
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-8" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <div
        ref={ref}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcut-sheet-title"
        className="w-full max-w-2xl rounded-lg border border-slate-700 bg-slate-900 p-5 text-sm text-slate-200 shadow-xl outline-none"
      >
        <div className="flex items-center justify-between">
          <h2 id="shortcut-sheet-title" className="text-base font-semibold">
            Keyboard shortcuts
          </h2>
          <CloseButton label="Close keyboard shortcuts" onClick={onClose} />
        </div>
        <div className="mt-4 grid grid-cols-3 gap-6">
          {groupedShortcuts().map(({ group, items }) => (
            <section key={group} aria-labelledby={`shortcut-group-${group}`}>
              <h3 id={`shortcut-group-${group}`} className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-400">
                {group}
              </h3>
              <dl className="space-y-1.5">
                {items.map((item) => (
                  <div key={item.id} className="flex items-center justify-between gap-3">
                    <dt className="text-slate-300">{item.label}</dt>
                    <dd className="flex shrink-0 gap-1">
                      {item.keys.map((key) => (
                        <kbd key={key} className="rounded border border-slate-600 bg-slate-800 px-1.5 py-0.5 font-mono text-xs text-slate-100">
                          {displayKey(key)}
                        </kbd>
                      ))}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </div>
    </div>
  );
}
