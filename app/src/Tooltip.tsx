import { cloneElement, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactElement } from "react";
import { createPortal } from "react-dom";

/** Hover delay before a tooltip shows; keyboard focus shows it at once. */
export const TOOLTIP_DELAY_MS = 350;

interface Props {
  /** What the control is, e.g. "Clear selection". Empty renders the control alone. */
  label: string | null | undefined;
  /** What it does or why you would use it. */
  detail?: string;
  /** One line each under the label, e.g. everyone in an avatar stack. */
  lines?: string[];
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

/** Gap between the control and the bubble, and the minimum margin to the window edge. */
const GAP = 6;
const EDGE = 8;

/** Only one bubble at a time, so a tooltip inside another one replaces it. */
let current: { close: () => void } | null = null;

/**
 * Places the bubble next to the control in viewport coordinates: flips to the
 * other side when it would leave the window and clamps it horizontally.
 */
export function placeBubble(
  anchor: { top: number; bottom: number; left: number; right: number },
  bubble: { width: number; height: number },
  view: { width: number; height: number },
  side: "top" | "bottom",
  align: "center" | "start" | "end",
): { top: number; left: number } {
  const below = anchor.bottom + GAP;
  const above = anchor.top - GAP - bubble.height;
  let top = side === "bottom" ? below : above;
  if (side === "bottom" && below + bubble.height > view.height - EDGE && above >= EDGE) {
    top = above;
  } else if (side === "top" && above < EDGE && below + bubble.height <= view.height - EDGE) {
    top = below;
  }
  let left =
    align === "start" ? anchor.left : align === "end" ? anchor.right - bubble.width : (anchor.left + anchor.right) / 2 - bubble.width / 2;
  left = Math.max(EDGE, Math.min(left, view.width - EDGE - bubble.width));
  return { top: Math.max(EDGE, top), left };
}

/**
 * One tooltip for every icon button (§8.2 "Tooltips"): shows on hover after a short delay
 * and on keyboard focus, hides on leave, blur or Escape, and is linked to the control
 * with `aria-describedby` so screen readers read it too. The bubble (and its hidden description while closed) is portaled to
 * `document.body` with fixed positioning so overflow containers never clip it, and it
 * flips or shifts to stay inside the window. The wrapper span takes the hover, so a
 * disabled button inside still gets its tooltip. Use this instead of the HTML `title`
 * attribute, which shows the OS tooltip (a lint rule enforces it).
 */
export function Tooltip({ label, detail, lines, shortcut, side = "bottom", align = "center", className = "", children }: Props) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLSpanElement>(null);
  const bubble = useRef<HTMLSpanElement>(null);
  const [position, setPosition] = useState<CSSProperties>({ top: 0, left: 0, visibility: "hidden" });
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

  useLayoutEffect(() => {
    if (!open || !anchor.current || !bubble.current) {
      return;
    }
    const place = () => {
      if (!anchor.current || !bubble.current) {
        return;
      }
      const box = bubble.current.getBoundingClientRect();
      const spot = placeBubble(anchor.current.getBoundingClientRect(), box, { width: window.innerWidth, height: window.innerHeight }, side, align);
      setPosition({ top: spot.top, left: spot.left });
    };
    place();
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
    };
  }, [open, side, align]);

  const close = () => {
    window.clearTimeout(timer.current);
    setOpen(false);
    setPosition({ top: 0, left: 0, visibility: "hidden" });
  };
  /** `close` only touches stable setters and refs, so the first one stays valid. */
  const self = useRef({ close });
  const openNow = () => {
    if (current && current !== self.current) {
      current.close();
    }
    current = self.current;
    setOpen(true);
  };
  const show = (delay: number) => {
    window.clearTimeout(timer.current);
    if (delay === 0) {
      openNow();
    } else {
      timer.current = window.setTimeout(openNow, delay);
    }
  };
  const hide = () => {
    if (current === self.current) {
      current = null;
    }
    close();
  };

  if (!label) {
    return children;
  }
  return (
    <span
      ref={anchor}
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
      {createPortal(
        open ? (
          <span
            ref={bubble}
            id={id}
            role="tooltip"
            data-side={side}
            style={{ position: "fixed", ...position }}
            className="pointer-events-none z-[1000] block w-max max-w-72 rounded-md border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-left text-xs font-normal normal-case tracking-normal break-words whitespace-normal text-slate-200 shadow-lg"
          >
            <Body label={label} detail={detail} lines={lines} shortcut={shortcut} />
          </span>
        ) : (
          <span id={id} role="tooltip" hidden>
            <Body label={label} detail={detail} lines={lines} shortcut={shortcut} />
          </span>
        ),
        document.body,
      )}
    </span>
  );
}

function Body({ label, detail, lines, shortcut }: Pick<Props, "label" | "detail" | "lines" | "shortcut">) {
  return (
    <>
      <span className="font-medium text-slate-100">{label}</span>
      {shortcut && <kbd className="ml-2 rounded border border-slate-600 px-1 font-mono text-[10px] text-slate-300">{shortcut}</kbd>}
      {detail && <span className="mt-0.5 block text-slate-400">{detail}</span>}
      {lines?.map((line, index) => (
        <span key={index} className="mt-0.5 block text-slate-300">
          {line}
        </span>
      ))}
    </>
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
