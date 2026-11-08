import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import {
  CloseIcon,
  CopyIcon,
  FolderPlusIcon,
  MoreIcon,
  SlackLogo,
} from "../icons";
import { Tooltip } from "../Tooltip";
import { hint } from "./shortcuts";

/** Below this list width the secondary actions move into the More menu. */
export const SELECTION_BAR_FULL_PX = 760;
/** Below this the count and the Slack button shorten too. */
export const SELECTION_BAR_COMPACT_PX = 480;

export type BarLayout = "full" | "more" | "compact";

/** The layout for the list column's width; 0 means "not measured yet". */
export function barLayout(width: number): BarLayout {
  if (width === 0 || width >= SELECTION_BAR_FULL_PX) {
    return "full";
  }
  return width >= SELECTION_BAR_COMPACT_PX ? "more" : "compact";
}

interface Props {
  count: number;
  /** Selected PRs hidden by the filters, not included. */
  hidden: number;
  addMenuOpen: boolean;
  onToggleAddMenu: () => void;
  /** The Add to project popover, shown above whichever button opened it. */
  addMenu: ReactNode;
  onRequest: () => void;
  onCopy: () => void;
  onClear: () => void;
  /** Opens the close confirmation for the active selection. */
  onClose: () => void;
}

const secondary =
  "flex items-center gap-2 rounded-md border border-slate-700 px-4 py-1.5 text-slate-200 hover:bg-slate-800";

/**
 * The floating bar under the list (§8.2). It is centered in the list column
 * and never wider than it, so it stays clear of the detail panel; as the
 * column narrows, Add to project and Copy as Markdown move into the More
 * menu, then the count and the Slack button shorten. More is always there:
 * on a wide column it holds only Close. Clear and the Slack button always
 * stay visible.
 */
export function SelectionBar({
  count,
  hidden,
  addMenuOpen,
  onToggleAddMenu,
  addMenu,
  onRequest,
  onCopy,
  onClose,
  onClear,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) {
      return;
    }
    setWidth(host.clientWidth);
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(() => setWidth(host.clientWidth));
    observer.observe(host);
    return () => observer.disconnect();
  }, []);

  const layout = barLayout(width);
  const folded = layout !== "full";

  useEffect(() => {
    if (!moreOpen) {
      return;
    }
    moreRef.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
    const onDown = (event: MouseEvent) => {
      if (!moreRef.current?.contains(event.target as Node)) {
        setMoreOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [moreOpen]);

  const onMenuKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [
      ...(moreRef.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ??
        []),
    ];
    const at = items.indexOf(document.activeElement as HTMLElement);
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      setMoreOpen(false);
      moreRef.current?.querySelector<HTMLElement>("[aria-haspopup]")?.focus();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      items[(at + step + items.length) % items.length]?.focus();
    }
  };

  const countText =
    layout === "compact"
      ? `${count}${hidden > 0 ? ` (+${hidden} hidden)` : ""}`
      : `${count} selected${hidden > 0 ? ` · ${hidden} hidden by filters${folded ? "" : ", not included"}` : ""}`;

  return (
    <div
      ref={hostRef}
      data-testid="selection-bar-host"
      className="pointer-events-none absolute inset-x-0 bottom-4 flex justify-center px-4"
    >
      <div
        role="region"
        aria-label="Selection actions"
        data-layout={layout}
        className={`pointer-events-auto flex max-w-full items-center rounded-lg border border-slate-700 bg-slate-900 text-sm shadow-lg ${layout === "compact" ? "gap-2 px-3 py-2" : "gap-4 px-5 py-2.5"}`}
      >
        <span
          aria-live="polite"
          className="min-w-0 truncate whitespace-nowrap text-slate-300"
        >
          {countText}
        </span>
        <span aria-hidden="true" className="h-6 w-px shrink-0 bg-slate-700" />
        <Tooltip
          label="Request / bump on Slack"
          shortcut={hint("request")}
          detail="Preview the review request for each selected PR before anything is sent"
          side="top"
        >
          <button
            type="button"
            aria-label="Request / bump on Slack"
            className={`flex shrink-0 items-center gap-2 rounded-md bg-violet-600 font-medium whitespace-nowrap text-white hover:bg-violet-500 disabled:bg-violet-500/40 ${layout === "compact" ? "px-3 py-1.5" : "px-4 py-1.5"}`}
            disabled={count === 0}
            onClick={onRequest}
          >
            <SlackLogo size={16} />
            {layout === "compact" ? "Slack" : "Request / bump on Slack"}
          </button>
        </Tooltip>
        {!folded && (
          <>
            <div className="relative shrink-0">
              <Tooltip
                label="Add to project"
                shortcut={hint("project")}
                detail="Link the selected PRs to a project, or start a new one with them"
                side="top"
              >
                <button
                  type="button"
                  aria-haspopup="menu"
                  aria-expanded={addMenuOpen}
                  className={secondary}
                  onClick={onToggleAddMenu}
                >
                  <FolderPlusIcon />
                  Add to project
                </button>
              </Tooltip>
              {addMenuOpen && addMenu}
            </div>
            <Tooltip
              label="Copy as Markdown"
              shortcut={hint("copyMarkdown")}
              detail="Copy the selected PRs as a Markdown list of links"
              side="top"
            >
              <button
                type="button"
                className={`${secondary} shrink-0`}
                disabled={count === 0}
                onClick={onCopy}
              >
                <CopyIcon />
                Copy as Markdown
              </button>
            </Tooltip>
          </>
        )}
        <div ref={moreRef} className="relative shrink-0" onKeyDown={onMenuKey}>
            <Tooltip
              label="More actions"
              detail={
                folded
                  ? "Add to project, Copy as Markdown, Close pull requests"
                  : "Close pull requests"
              }
              side="top"
            >
              <button
                type="button"
                aria-label="More actions"
                aria-haspopup="menu"
                aria-expanded={moreOpen}
                className="flex items-center rounded-md border border-slate-700 p-2 text-slate-200 hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-violet-400"
                onClick={() => setMoreOpen((open) => !open)}
              >
                <MoreIcon />
              </button>
            </Tooltip>
            {moreOpen && (
              <div
                role="menu"
                aria-label="More actions"
                className="absolute bottom-full left-1/2 z-20 mb-2 w-max min-w-48 -translate-x-1/2 whitespace-nowrap rounded border border-slate-700 bg-slate-900 py-1 text-sm shadow-lg"
              >
                {folded && (
                  <>
                    <button
                      type="button"
                      role="menuitem"
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-slate-200 hover:bg-slate-800 focus:bg-slate-800 focus:outline-none"
                      onClick={() => {
                        setMoreOpen(false);
                        onToggleAddMenu();
                      }}
                    >
                      <FolderPlusIcon />
                      <span className="flex-1">Add to project</span>
                      <kbd className="text-xs text-slate-500">
                        {hint("project")}
                      </kbd>
                    </button>
                    <button
                      type="button"
                      role="menuitem"
                      disabled={count === 0}
                      className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-slate-200 hover:bg-slate-800 focus:bg-slate-800 focus:outline-none disabled:text-slate-500"
                      onClick={() => {
                        setMoreOpen(false);
                        onCopy();
                      }}
                    >
                      <CopyIcon />
                      <span className="flex-1">Copy as Markdown</span>
                      <kbd className="text-xs text-slate-500">
                        {hint("copyMarkdown")}
                      </kbd>
                    </button>
                    <div role="separator" className="my-1 h-px bg-slate-800" />
                  </>
                )}
                <button
                  type="button"
                  role="menuitem"
                  disabled={count === 0}
                  className="flex w-full items-center gap-2 px-3 py-1.5 text-left text-rose-300 hover:bg-slate-800 focus:bg-slate-800 focus:outline-none disabled:text-slate-500"
                  onClick={() => {
                    setMoreOpen(false);
                    onClose();
                  }}
                >
                  <span className="flex-1">
                    {count === 1 ? "Close PR" : `Close ${count} PRs`}
                  </span>
                </button>
              </div>
            )}
            {folded && addMenuOpen && addMenu}
          </div>
        <Tooltip
          label="Clear selection"
          shortcut={hint("clear")}
          detail="Unselect every PR in this view"
          side="top"
        >
          <button
            type="button"
            aria-label="Clear selection"
            className="flex shrink-0 items-center rounded-md border border-slate-700 p-2 text-slate-200 hover:bg-slate-800 focus-visible:outline-2 focus-visible:outline-violet-400"
            onClick={onClear}
          >
            <CloseIcon />
          </button>
        </Tooltip>
      </div>
    </div>
  );
}
