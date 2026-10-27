import type { Ref } from "react";
import { CloseIcon } from "./icons";
import { Tooltip } from "./Tooltip";

/** The icon-only ✕ every dialog, banner and toast uses, with its tooltip. */
export function CloseButton({
  label = "Close",
  detail,
  shortcut = "Esc",
  onClick,
  ref,
  className = "",
  tone = "text-slate-400 hover:text-slate-100",
}: {
  /** Accessible name, e.g. "Dismiss update notice". */
  label?: string;
  detail?: string;
  shortcut?: string | null;
  onClick: () => void;
  ref?: Ref<HTMLButtonElement>;
  /** Classes for the wrapper, e.g. `ml-auto`. */
  className?: string;
  /** Text color classes for the icon. */
  tone?: string;
}) {
  return (
    <Tooltip label={label} detail={detail} shortcut={shortcut ?? undefined} align="end" className={className}>
      <button
        ref={ref}
        type="button"
        aria-label={label}
        className={`flex h-7 w-7 items-center justify-center rounded hover:bg-slate-800 ${tone}`}
        onClick={onClick}
      >
        <CloseIcon />
      </button>
    </Tooltip>
  );
}
