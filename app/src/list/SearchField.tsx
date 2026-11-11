import { useEffect, useId, useState, type RefObject } from "react";
import { CloseIcon, SearchIcon } from "../icons";
import { Tooltip } from "../Tooltip";
import { hint } from "./shortcuts";
import { replaceToken, suggestionsFor, tokenAt, type Suggestion } from "./searchText";
import { plainText } from "../textInput";

interface Props {
  value: string;
  onChange: (value: string) => void;
  searching?: boolean;
  inputRef: RefObject<HTMLInputElement | null>;
  /** `toolbar` stretches in the list header. `dialog` fills the view dialog. */
  layout?: "toolbar" | "dialog";
  /** Accessible name. The toolbar field is Search; the view dialog is Query. */
  label?: string;
}

/**
 * The one list search box. Qualifiers and free text share it; completion is
 * a short list while a qualifier is being typed.
 */
export function SearchField({ value, onChange, searching = false, inputRef, layout = "toolbar", label = "Search" }: Props) {
  const listId = useId();
  const hintId = useId();
  const [focused, setFocused] = useState(false);
  const [caret, setCaret] = useState(value.length);
  const [active, setActive] = useState(0);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [pendingCaret, setPendingCaret] = useState<number | null>(null);
  const token = tokenAt(value, caret);
  const tokenText = token?.text ?? "";
  const [tracked, setTracked] = useState(tokenText);
  if (tracked !== tokenText) {
    setTracked(tokenText);
    setActive(0);
    setDismissed(null);
  }
  const open = focused && token !== null && dismissed !== tokenText;
  const suggestions = open ? suggestionsFor(tokenText).slice(0, 6) : [];
  const current = suggestions.length === 0 ? 0 : Math.min(active, suggestions.length - 1);

  function rememberCaret(element: HTMLInputElement) {
    setCaret(element.selectionStart ?? element.value.length);
  }

  function apply(suggestion: Suggestion) {
    if (!token) {
      return;
    }
    const next = replaceToken(value, token, suggestion.insert);
    setPendingCaret(next.caret);
    setCaret(next.caret);
    onChange(next.value);
  }

  useEffect(() => {
    if (pendingCaret === null || !inputRef.current) {
      return;
    }
    inputRef.current.setSelectionRange(pendingCaret, pendingCaret);
    setPendingCaret(null);
  }, [pendingCaret, inputRef]);

  const toolbar = layout === "toolbar";
  return (
    <div className={toolbar ? "relative min-w-0 flex-[2]" : "relative w-full"}>
      <label className="flex h-8 w-full min-w-0 items-center gap-1.5 rounded-md border border-slate-700 bg-slate-900 px-2 text-sm focus-within:border-violet-400">
        <Tooltip
          label="Search or filter"
          shortcut={toolbar ? hint("search") : undefined}
          detail="Qualifiers such as author:, is:, and sort: search GitHub. bump:due stays on this machine. Other words search titles, projects, repos, and numbers."
          align="start"
        >
          <span className="inline-flex shrink-0 text-slate-500">
            <SearchIcon />
          </span>
        </Tooltip>
        <span id={hintId} className="sr-only">
          Qualifiers such as author:me, is:draft, and sort:updated-desc search GitHub. bump:due is decided on this machine and is not sent to GitHub. Other words search this list.
        </span>
        <input
          {...plainText}
          ref={inputRef}
          aria-label={label}
          aria-autocomplete="list"
          aria-expanded={suggestions.length > 0}
          aria-controls={suggestions.length > 0 ? listId : undefined}
          aria-activedescendant={suggestions.length > 0 ? `${listId}-opt-${current}` : undefined}
          aria-describedby={hintId}
          className={`min-w-0 flex-1 bg-transparent outline-none placeholder:text-slate-500 ${toolbar ? "placeholder:text-transparent @3xl:placeholder:text-slate-500" : ""}`}
          placeholder="Search or filter, e.g. author:me is:draft"
          value={value}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onSelect={(event) => rememberCaret(event.currentTarget)}
          onChange={(event) => {
            rememberCaret(event.currentTarget);
            onChange(event.target.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape" && suggestions.length > 0) {
              event.preventDefault();
              event.stopPropagation();
              setDismissed(tokenText);
              return;
            }
            if (suggestions.length === 0) {
              return;
            }
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActive((index) => (index + 1) % suggestions.length);
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setActive((index) => (index - 1 + suggestions.length) % suggestions.length);
            } else if (event.key === "Enter") {
              event.preventDefault();
              const suggestion = suggestions[current];
              if (suggestion) {
                apply(suggestion);
              }
            }
          }}
        />
        {searching && (
          <span role="status" className="sr-only shrink-0 text-xs text-slate-500 @3xl:not-sr-only @3xl:inline">
            Searching…
          </span>
        )}
        {value !== "" && (
          <button
            type="button"
            aria-label="Clear search"
            className="shrink-0 text-slate-500 hover:text-slate-200"
            onClick={() => onChange("")}
          >
            <CloseIcon />
          </button>
        )}
      </label>
      {suggestions.length > 0 && (
        <ul
          id={listId}
          role="listbox"
          aria-label="Search suggestions"
          className="absolute left-0 top-8 z-30 w-72 max-w-[min(18rem,calc(100vw-2rem))] rounded-md border border-slate-700 bg-slate-900 py-1 text-sm shadow-xl"
        >
          {suggestions.map((suggestion, index) => (
            <li key={suggestion.insert} role="presentation">
              <button
                type="button"
                id={`${listId}-opt-${index}`}
                role="option"
                aria-selected={index === current}
                className={`flex w-full items-baseline gap-2 px-2 py-1 text-left ${index === current ? "bg-slate-800 text-slate-100" : "text-slate-300"}`}
                onMouseDown={(event) => event.preventDefault()}
                onMouseEnter={() => setActive(index)}
                onClick={() => apply(suggestion)}
              >
                <span className="font-medium text-slate-100">{suggestion.label}</span>
                {suggestion.hint !== "" && <span className="truncate text-xs text-slate-500">{suggestion.hint}</span>}
              </button>
            </li>
          ))}
          <li className="border-t border-slate-800 px-2 py-1 text-xs text-slate-500">Other words search this list.</li>
        </ul>
      )}
    </div>
  );
}
