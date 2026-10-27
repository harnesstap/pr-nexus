import { cloneElement, useEffect, useId, useRef, useState, type ReactElement } from "react";

/** Hover delay before a tooltip shows; keyboard focus shows it at once. */
export const TOOLTIP_DELAY_MS = 350;

interface Props {
  /** What the control is, e.g. "Clear selection". */
  label: string;
  /** What it does or why you would use it. */
  detail?: string;
  /** Keyboard shortcut shown after the label. */
  shortcut?: string;
  /** Where the bubble goes relative to the control. */
  side?: "top" | "bottom";
  /** Pins the bubble to one edge instead of centering it. */
  align?: "center" | "start" | "end";
  /** Classes for the wrapper, e.g. `ml-auto` when the control sits in a flex row. */
  className?: string;
  children: ReactElement<{ "aria-describedby"?: string }>;
}

const SIDE = { top: "bottom-full mb-1.5", bottom: "top-full mt-1.5" };
const ALIGN = { center: "left-1/2 -translate-x-1/2", start: "left-0", end: "right-0" };

/**
 * One tooltip for every icon button (§8.2 "Tooltips"): shows on hover after a short delay
 * and on keyboard focus, hides on leave, blur or Escape, and is linked to the control
 * with `aria-describedby` so screen readers read it too.
 */
export function Tooltip({ label, detail, shortcut, side = "bottom", align = "center", className = "", children }: Props) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  /** A click focuses the control too; only keyboard focus should open the bubble. */
  const pointer = useRef(false);

  useEffect(() => () => window.clearTimeout(timer.current), []);
  useEffect(() => {
    if (!open) {
      return;
    }
    const onKey = (event: KeyboardEvent) => event.key === "Escape" && setOpen(false);
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  const show = (delay: number) => {
    window.clearTimeout(timer.current);
    if (delay === 0) {
      setOpen(true);
    } else {
      timer.current = window.setTimeout(() => setOpen(true), delay);
    }
  };
  const hide = () => {
    window.clearTimeout(timer.current);
    setOpen(false);
  };

  return (
    <span
      className={`relative inline-flex ${className}`}
      onMouseEnter={() => show(TOOLTIP_DELAY_MS)}
      onMouseLeave={hide}
      onFocus={() => !pointer.current && keyboardFocus() && show(0)}
      onBlur={() => {
        pointer.current = false;
        hide();
      }}
      onPointerDown={() => {
        pointer.current = true;
        hide();
      }}
    >
      {cloneElement(children, { "aria-describedby": id })}
      <span
        id={id}
        role="tooltip"
        hidden={!open}
        className={`pointer-events-none absolute z-50 w-max max-w-64 rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-left text-xs font-normal normal-case tracking-normal text-slate-200 shadow-lg ${SIDE[side]} ${ALIGN[align]}`}
      >
        <span className="font-medium text-slate-100">{label}</span>
        {shortcut && <kbd className="ml-2 rounded border border-slate-600 px-1 font-mono text-[10px] text-slate-300">{shortcut}</kbd>}
        {detail && <span className="mt-0.5 block text-slate-400">{detail}</span>}
      </span>
    </span>
  );
}

/**
 * Dialogs focus their Close button on open; that should only pop a bubble when the
 * user is moving around with the keyboard. WKWebView's `:focus-visible` is not
 * reliable for programmatic focus, so track the last input kind ourselves.
 */
let lastInput: "keyboard" | "pointer" = "pointer";
if (typeof document !== "undefined") {
  document.addEventListener("keydown", () => (lastInput = "keyboard"), true);
  document.addEventListener("pointerdown", () => (lastInput = "pointer"), true);
  document.addEventListener("mousedown", () => (lastInput = "pointer"), true);
}

function keyboardFocus(): boolean {
  return lastInput === "keyboard";
}
