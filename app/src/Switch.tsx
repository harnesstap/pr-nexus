import type { ReactNode } from "react";

interface Props {
  /** Accessible name, e.g. "Live updates" or "Use Acme". */
  label: string;
  checked: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
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
export function Switch({ label, checked, onChange, disabled = false, children, className = "", "aria-describedby": describedBy }: Props) {
  return (
    <span className={`inline-flex items-center gap-2 ${className}`}>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        aria-describedby={describedBy}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-violet-300 disabled:opacity-50 ${
          checked ? "bg-violet-500" : "bg-slate-600"
        }`}
      >
        <span
          aria-hidden="true"
          className={`inline-block h-4 w-4 rounded-full bg-white shadow transition-transform ${checked ? "translate-x-4" : "translate-x-0.5"}`}
        />
      </button>
      {children && (
        <span className={`text-sm ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`} onClick={() => !disabled && onChange(!checked)}>
          {children}
        </span>
      )}
    </span>
  );
}
