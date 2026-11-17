import type { ReactNode } from "react";

interface Props {
  /** Accessible name, e.g. "Live updates" or "Use Acme". */
  label: string;
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
  /** A shorter track for a row inside a list. */
  compact?: boolean;
  /** Visible text next to the switch; omit when the label sits elsewhere. */
  children?: ReactNode;
  className?: string;
  /** Set by Tooltip. */
  "aria-describedby"?: string;
}

/**
 * The one on/off control (§8.2): a `role="switch"` button, so Space and Enter
 * toggle it and screen readers say "on" or "off". Use it for every boolean setting.
 */
export function Switch({ label, checked, onChange, disabled = false, compact = false, children, className = "", "aria-describedby": describedBy }: Props) {
  const track = compact ? "h-3.5 w-6" : "h-5 w-9";
  const knob = compact ? "h-2.5 w-2.5" : "h-4 w-4";
  const travel = compact ? "translate-x-3" : "translate-x-4";
  return (
    <span className={`flex w-fit max-w-full items-start gap-2 ${className}`}>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        aria-describedby={describedBy}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative inline-flex ${track} shrink-0 items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-300 disabled:opacity-50 ${
          checked ? "bg-violet-500" : "bg-slate-600"
        }`}
      >
        <span
          aria-hidden="true"
          className={`inline-block ${knob} rounded-full bg-white shadow transition-transform ${checked ? travel : "translate-x-0.5"}`}
        />
      </button>
      {children && (
        <span className={`min-w-0 text-sm ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`} onClick={() => !disabled && onChange(!checked)}>
          {children}
        </span>
      )}
    </span>
  );
}
