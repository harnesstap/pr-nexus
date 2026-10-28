import { useEffect, useId, useState } from "react";
import type { Api, ReviewTarget, SlackWorkspace, TargetChoice, TargetMatch } from "../api";
import { channelLabel, errorMessage } from "../api";
import { plainText } from "../textInput";
import { targetWorkspaceName } from "./workspaces";

/** Wait after the last keystroke before asking Slack (§7.11). */
export const SEARCH_DEBOUNCE_MS = 250;

interface Props {
  api: Pick<Api, "searchSlackTargets">;
  workspaces: SlackWorkspace[];
  /** Code-owner channels and open requests, listed before any search result. */
  suggestions: TargetChoice[];
  value: ReviewTarget | null;
  /** `choice` is the suggestion picked, or null for a search result or an ID. */
  onSelect: (target: ReviewTarget, choice: TargetChoice | null) => void;
  label?: string;
}

type Option =
  | { key: string; group: "suggested"; target: ReviewTarget; choice: TargetChoice }
  | { key: string; group: "channel" | "member"; target: ReviewTarget }
  | { key: string; group: "id"; target: ReviewTarget };

interface Found {
  items: TargetMatch[];
  error: string | null;
}

const GROUP_LABEL = {
  suggested: "Code owners and open requests",
  channel: "Channels",
  member: "People · direct message",
  id: "Slack ID",
} as const;

const ID_PATTERN = /^[CGDUW][A-Z0-9]{8,}$/i;

/** `#term` → channels, `@term` → people, a bare term → both. */
export function parseQuery(query: string): { channels: boolean; members: boolean; term: string } {
  const trimmed = query.trim();
  if (trimmed.startsWith("#")) {
    return { channels: true, members: false, term: trimmed.slice(1).trim() };
  }
  if (trimmed.startsWith("@")) {
    return { channels: false, members: true, term: trimmed.slice(1).trim() };
  }
  return { channels: true, members: true, term: trimmed };
}

/** Where a search result sorts: what the prefix asks for first, then workspace order, then names starting with the term. */
export function rankMatches(matches: TargetMatch[], query: string, workspaceOrder: string[]): TargetMatch[] {
  const trimmed = query.trim();
  const peopleFirst = trimmed.startsWith("@");
  const term = parseQuery(trimmed).term.toLowerCase();
  const kindRank = (kind: TargetMatch["kind"]) => ((kind === "member") === peopleFirst ? 0 : 1);
  const wsRank = (workspace: string) => {
    const at = workspaceOrder.indexOf(workspace);
    return at === -1 ? workspaceOrder.length : at;
  };
  const prefixRank = (name: string) => {
    const lower = name.toLowerCase();
    if (!term || lower.startsWith(term)) {
      return 0;
    }
    // A later word starting with the term ("Grace Hopper" for "hop") beats a mid-word hit.
    return lower.split(/[\s\-_.]+/).some((word) => word.startsWith(term)) ? 1 : 2;
  };
  return matches
    .map((match, index) => ({ match, index, key: [kindRank(match.kind), wsRank(match.target.workspace), prefixRank(match.target.channel_name)] }))
    .sort((a, b) => a.key[0] - b.key[0] || a.key[1] - b.key[1] || a.key[2] - b.key[2] || a.index - b.index)
    .map((item) => item.match);
}

/** The header an option sits under: one per kind, split by workspace when several are searched. */
function sectionOf(option: Option, several: boolean): { key: string; label: string } {
  if (several && (option.group === "channel" || option.group === "member")) {
    const name = targetWorkspaceName(option.target);
    return { key: `${option.group}:${option.target.workspace}`, label: `${GROUP_LABEL[option.group]} · ${name}` };
  }
  return { key: option.group, label: GROUP_LABEL[option.group] };
}

/** Channel IDs are unique across a grid, so the ID alone dedupes. */
function keyOf(target: ReviewTarget) {
  return target.channel_id;
}

function describe(choice: TargetChoice) {
  if (choice.source === "request") {
    return "open request";
  }
  return choice.teams.join(", ");
}

/**
 * Select2-style Slack target picker (§7.11, §8.2): suggestions first, then a
 * debounced MCP search of every enabled workspace, in priority order, merged
 * and labeled with the workspace. ARIA combobox with a listbox;
 * "Enter an ID" stays available when search is unsupported or fails.
 */
export function TargetCombobox({ api, workspaces, suggestions, value, onSelect, label = "Channel or person" }: Props) {
  const id = useId();
  const listId = `${id}-list`;
  const helpId = `${id}-help`;
  const [query, setQuery] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [found, setFound] = useState<Record<string, Found>>({});
  const [manual, setManual] = useState(false);
  const [manualId, setManualId] = useState("");
  const [manualName, setManualName] = useState("");

  const enabled = workspaces.filter((item) => item.enabled !== false);
  const several = enabled.length > 1;
  const text = query ?? (value ? channelLabel(value) : "");
  const parsed = parseQuery(query ?? "");
  const searchKey = parsed.term ? (query ?? "").trim() : "";
  const result = searchKey ? found[searchKey] : undefined;
  const searching = searchKey !== "" && enabled.length > 0 && result === undefined;

  useEffect(() => {
    if (!searchKey || enabled.length === 0 || found[searchKey]) {
      return;
    }
    let cancel = false;
    const wanted = (query ?? "").trim();
    const timer = window.setTimeout(() => {
      api
        .searchSlackTargets(wanted)
        .then((items) => ({ items, error: null }))
        .catch((err: unknown) => ({ items: [], error: errorMessage(err) }))
        .then((next) => {
          if (!cancel) {
            setFound((current) => ({ ...current, [searchKey]: next }));
          }
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancel = true;
      window.clearTimeout(timer);
    };
  }, [api, found, query, searchKey, enabled.length]);

  const term = parsed.term.toLowerCase();
  const options: Option[] = [];
  const seen = new Set<string>();
  for (const choice of suggestions) {
    const name = choice.target.channel_name.toLowerCase();
    if (term && !name.includes(term) && !choice.teams.some((team) => team.toLowerCase().includes(term))) {
      continue;
    }
    seen.add(keyOf(choice.target));
    options.push({ key: keyOf(choice.target), group: "suggested", target: choice.target, choice });
  }
  const order = enabled.map((item) => item.domain);
  for (const match of rankMatches(result?.items ?? [], query ?? "", order)) {
    const key = keyOf(match.target);
    if (!seen.has(key)) {
      seen.add(key);
      options.push({ key, group: match.kind, target: match.target });
    }
  }
  if (ID_PATTERN.test(parsed.term)) {
    // No workspace: the send probes enabled workspaces in priority order.
    const target: ReviewTarget = { workspace: "", channel_id: parsed.term.toUpperCase(), channel_name: "" };
    if (!seen.has(keyOf(target))) {
      options.push({ key: `id:${keyOf(target)}`, group: "id", target });
    }
  }
  const current = Math.min(active, Math.max(options.length - 1, 0));
  const optionId = (index: number) => `${id}-option-${index}`;

  function choose(option: Option) {
    setQuery(null);
    setOpen(false);
    setActive(0);
    onSelect(option.target, option.group === "suggested" ? option.choice : null);
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive(options.length ? (current + step + options.length) % options.length : 0);
    } else if (event.key === "Enter") {
      if (open && options[current]) {
        event.preventDefault();
        choose(options[current]);
      }
    } else if (event.key === "Escape") {
      if (open || query !== null) {
        // Close the list, not the dialog around it.
        event.stopPropagation();
        event.preventDefault();
        setOpen(false);
        setQuery(null);
      }
    } else if (event.key === "Tab") {
      setOpen(false);
    }
  }

  function useManual() {
    const channelId = manualId.trim().toUpperCase();
    if (!channelId) {
      return;
    }
    onSelect({ workspace: "", channel_id: channelId, channel_name: manualName.trim() }, null);
  }

  const input = "mt-1 w-full rounded border border-slate-700 bg-slate-950 px-2 py-1.5";
  let lastSection: string | null = null;
  return (
    <div className="space-y-1">
      <div>
        <div className="relative">
          <label className="block" htmlFor={`${id}-input`}>
            {label}
          </label>
          <input
            {...plainText}
            id={`${id}-input`}
            className={input}
            role="combobox"
            aria-expanded={open}
            aria-controls={listId}
            aria-autocomplete="list"
            aria-describedby={helpId}
            aria-activedescendant={open && options[current] ? optionId(current) : undefined}
            placeholder="#channel or @person"
            value={text}
            onFocus={(event) => {
              event.currentTarget.select();
              setOpen(true);
            }}
            onClick={() => setOpen(true)}
            onBlur={() => {
              setOpen(false);
              setQuery(null);
            }}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
              setOpen(true);
            }}
            onKeyDown={onKeyDown}
          />
          {open && (
            <ul
              id={listId}
              role="listbox"
              aria-label={label}
              className="absolute z-50 mt-1 max-h-60 w-full overflow-auto rounded border border-slate-700 bg-slate-900 py-1 shadow-xl"
            >
              {options.map((option, index) => {
                const section = sectionOf(option, several);
                const header = section.key !== lastSection ? section.label : null;
                lastSection = section.key;
                return [
                  header && (
                    <li
                      key={`h-${section.key}`}
                      role="presentation"
                      className="sticky top-0 bg-slate-900 px-2 pb-0.5 pt-1.5 text-[11px] font-medium uppercase tracking-wide text-slate-500"
                    >
                      {header}
                    </li>
                  ),
                  <li
                    key={option.key}
                    id={optionId(index)}
                    role="option"
                    aria-selected={index === current}
                    className={`flex cursor-pointer items-baseline gap-2 px-2 py-1 ${index === current ? "bg-violet-500/20" : ""}`}
                    onMouseDown={(event) => event.preventDefault()}
                    onMouseEnter={() => setActive(index)}
                    onClick={() => choose(option)}
                  >
                    <span>{option.group === "id" ? `Use ${option.target.channel_id}` : channelLabel(option.target)}</span>
                    <span className="truncate text-xs text-slate-500">
                      {[
                        option.group === "suggested" ? describe(option.choice) : option.group === "member" ? "direct message" : "",
                        several && option.group === "suggested" ? targetWorkspaceName(option.target) : "",
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </span>
                  </li>,
                ];
              })}
              {searching && (
                <li role="presentation" className="px-2 py-1 text-xs text-slate-400">
                  Searching {several ? `${enabled.length} workspaces` : targetWorkspaceName({ workspace: enabled[0]?.domain ?? "", workspace_name: enabled[0]?.team_name })}…
                </li>
              )}
              {result?.error && (
                <li role="presentation" className="px-2 py-1 text-xs text-amber-300">
                  Search failed: {result.error}
                </li>
              )}
              {!searching && !result?.error && options.length === 0 && (
                <li role="presentation" className="px-2 py-1 text-xs text-slate-400">
                  {parsed.term ? "No matches." : "Type # for channels or @ for people."}
                </li>
              )}
            </ul>
          )}
        </div>
      </div>
      <p id={helpId} className="text-xs text-slate-500">
        Type # to search channels or @ to search people; a person gets a direct message. Code-owner channels are listed
        first{several ? ", then results by workspace" : ""}.{" "}
        <button type="button" className="underline" onClick={() => setManual((shown) => !shown)} aria-expanded={manual}>
          {manual ? "Hide ID entry" : "Enter an ID"}
        </button>
      </p>
      {manual && (
        <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-end gap-2">
          <label className="block">
            Channel, DM or member ID
            <input
              {...plainText}
              className={input}
              placeholder="C0123456789"
              value={manualId}
              onChange={(event) => setManualId(event.target.value)}
            />
          </label>
          <label className="block">
            Name
            <input
              {...plainText}
              className={input}
              placeholder="#reviews"
              value={manualName}
              onChange={(event) => setManualName(event.target.value)}
            />
          </label>
          <button type="button" className="rounded border border-slate-600 px-3 py-1.5" onClick={useManual}>
            Preview
          </button>
        </div>
      )}
    </div>
  );
}
