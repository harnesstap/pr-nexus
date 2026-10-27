import { useId, useState, type ReactNode } from "react";
import { ChevronDownIcon, ChevronRightIcon } from "../icons";

/** A collapsed section with a one-line header; the body mounts only when opened. */
export function Expander({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <section className="border-t border-slate-800">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 py-2.5 text-left text-sm text-slate-300 hover:text-slate-100"
      >
        {open ? <ChevronDownIcon size={14} /> : <ChevronRightIcon size={14} />}
        <span className="font-medium">{title}</span>
        {aside && <span className="ml-auto text-xs text-slate-500">{aside}</span>}
      </button>
      {open && (
        <div id={id} className="pb-3">
          {children}
        </div>
      )}
    </section>
  );
}
